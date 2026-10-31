//! The client side of a match's life: setting it up from what was loaded,
//! swapping it for the next, and what the player sees in between.

mod entry_gate;
pub mod bo2mp_setup;
pub mod fight_table;
pub mod content_ledger;
pub mod lives;
pub mod match_cycle;
pub mod loadout;
pub mod self_arm;
pub mod match_install;
pub mod wiring;
pub mod startup;
pub mod eye_target;

pub use bo2mp_setup::Bo2mpMatchSetup;
pub use content_ledger::{
    HostArmamentLoadout, ManifestEntryState, ContentShortfall, LiveRuleVariant, GameContentLedger,
    ContentManifestFault, ManifestGunId, ManifestGunEntry,
};
pub use lives::{SpawnWaveTally, SpawnAlertTally};
pub use match_cycle::{
    RunningWorldLabel, HandoverFinish, SessionHandoverOrder, HandoverVerdict,
    SessionReplacementGoal, ShutdownLeftovers, ShutdownOrder,
};
pub use loadout::{
    HostLoadoutMirror, ClassRow, host_loadout_lock_cause, class_perk_slots,
    fill_default_perks, perk_item_number, project_loadout, resolve_class_weapon,
};
pub use match_install::{install_staged_game, scripted_prop_id};
pub use wiring::LobbyLifecyclePlugin;
pub use startup::{StagedConsoleText, BootConsoleScript};
