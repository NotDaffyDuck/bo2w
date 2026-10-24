//! Per-client match state and what the server publishes about it each snapshot.

mod audience;
mod blocker;
mod client;
mod client_meta;
mod dvars;
mod enemy_model;
mod loadout;
mod menu;
mod meta_sync;
mod pickups;
mod radar;
mod reasons;
mod records;
mod remote;
mod seat;
mod sim_event;
mod snapshot_meta;
mod view;

pub use audience::EventAudience;
pub use blocker::OrientedBlocker;
pub use client::{ClientMatchState, InputReceipt, ScriptControls};
pub use client_meta::{ClientSnapshotMeta, T6ViewInputs};
pub use dvars::ScriptDvars;
pub use enemy_model::EnemyModel;
pub use loadout::{
    CLASS_CATALOG_DEATHSTREAKS, CLASS_CATALOG_PERKS, ClassDef, ClassRejectReason,
    ConfigurationChangeRejectReason, GiveRejectReason, LoadoutSpec, PERSONAL_CLASS_SLOTS, PersonalClass,
};
pub use menu::{LocationSelection, MENU_COMMAND_TAIL, MenuCommand, MenuCommandKind};
pub use pickups::{DroppedItemAmmo, ItemPickupRecord};
pub use radar::RadarMode;
pub use records::{DebrisRecord, EntityEventPayload, EntityEventRecord, EventRecord, PelletFxRecord};
pub use remote::RemoteMissile;
pub use seat::{KillcamHud, ScriptSeat};
pub use sim_event::{SIM_EVENT_ROSTER, SimEvent, SimEventRow, sim_event_is_reliable};
pub use snapshot_meta::{RngDebugMeta, SnapshotMeta};
pub use view::{ScriptBlur, ScriptDepthOfField, ViewEffects, VisionChange, is_postfx_dvar};

/// Where a client is between connecting and the end of the match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClientLifecycle {
    #[default]
    Connecting,
    ChoosingClass,
    SpawnPending,
    Alive,
    Dead,
    RespawnPending,
    Spectating,
    Intermission,
}

/// What ended the match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchEndReason {
    ScoreLimit,
    TimeLimit,
}

/// The name of perk `id` in the class catalog (ids count from 1).
pub fn class_catalog_perk_name(id: u32) -> Option<&'static str> {
    let index = id.checked_sub(1)?;
    CLASS_CATALOG_PERKS.get(index as usize).copied()
}

/// Which of the three perk slots catalog perk `id` goes in.
pub fn perk_slot_from_class_catalog(id: u32) -> Option<usize> {
    const SLOT_ONE: [u32; 5] = [2, 5, 8, 12, 13];
    const SLOT_TWO: [u32; 5] = [1, 3, 4, 6, 9];
    const SLOT_THREE: [u32; 6] = [7, 10, 11, 14, 15, 16];
    if SLOT_ONE.contains(&id) {
        Some(0)
    } else if SLOT_TWO.contains(&id) {
        Some(1)
    } else if SLOT_THREE.contains(&id) {
        Some(2)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_perk_lands_in_one_slot() {
        let mut per_slot = [0; 3];
        for id in 1..=16 {
            per_slot[perk_slot_from_class_catalog(id).expect("catalog id has a slot")] += 1;
        }
        assert_eq!(per_slot, [5, 5, 6]);
        assert_eq!(perk_slot_from_class_catalog(0), None);
        assert_eq!(perk_slot_from_class_catalog(17), None);
    }

    #[test]
    fn perk_names_count_from_one() {
        assert_eq!(class_catalog_perk_name(0), None);
        assert_eq!(class_catalog_perk_name(1), Some(CLASS_CATALOG_PERKS[0]));
        assert_eq!(class_catalog_perk_name(16), Some(CLASS_CATALOG_PERKS[15]));
        assert_eq!(class_catalog_perk_name(17), None);
    }
}
