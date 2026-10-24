//! Entity numbers. The kernel hands numbers out and takes them back, stamps each reuse so an old handle is
//! recognised as stale, and keeps the per-frame bookkeeping (who already thought, which events have expired).
//! Beside it: the little state builders for the entity kinds the sim spawns itself, and the use-key edge detector.

mod kernel;
mod motion;
mod presses;
mod refs;
mod snapshot;

#[cfg(test)]
mod tests;

use playerstate_t6::game::GENTITY_SPAWN_BASE;

pub use kernel::EntityKernel;
pub use motion::{
    ScriptMoverGentity, apos_from_entity_state, begin_script_mover_rotate_velocity, init_item_state,
    init_missile_state, init_script_mover_state, pos_from_entity_state, rotate_velocity_apos,
};
pub use presses::{UsePress, collect_use_presses};
pub use refs::{
    EntityAllocError, EntityRef, EntityRefError, EntityRelations, EntitySlotView, ThinkEnter,
};
pub use snapshot::{
    EntityKernelOccupiedSnapshot, EntityKernelSlotSnapshot, EntityKernelSnapshot, EntityKernelSnapshotError,
};

/// Numbers below this belong to clients and corpses; the kernel never hands them out.
pub const GENTITY_RESERVED_COUNT: i32 = GENTITY_SPAWN_BASE;
/// A freed number is not given out again until this long after it was freed (unless the table is full).
pub const GENTITY_REUSE_QUARANTINE_MS: i32 = 500;
/// How long a one-shot event entity lives before the kernel frees it.
pub const GENTITY_TEMP_EVENT_LIFETIME_MS: i32 = 300;

/// The parts of one authority step, in the order they must run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum KernelPhase {
    AdvanceTime,
    ExpireTransientEvents,
    ApplyActions,
    RunPlayers,
    RecordCollisionState,
    RunEntityTypes,
    DispatchTouches,
    Finalize,
    PublishSnapshot,
}

pub const KERNEL_PHASE_ORDER: &[KernelPhase] = &[
    KernelPhase::AdvanceTime,
    KernelPhase::ExpireTransientEvents,
    KernelPhase::ApplyActions,
    KernelPhase::RunPlayers,
    KernelPhase::RecordCollisionState,
    KernelPhase::RunEntityTypes,
    KernelPhase::DispatchTouches,
    KernelPhase::Finalize,
    KernelPhase::PublishSnapshot,
];

/// What runs in an entity's slot each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntityRunKind {
    ScriptMover,
    Item,
    Missile,
    TempEvent,
    PrimaryLight,
    General,
    PlayerCorpse,
}

/// The first number the kernel hands out (callers outside the sim ask for it by function).
pub fn gentity_spawn_base() -> i32 {
    GENTITY_SPAWN_BASE
}
