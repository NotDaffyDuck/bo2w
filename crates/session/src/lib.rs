//! The client side of a match's life: setting it up from what was loaded,
//! swapping it for the next, and what the player sees in between.

mod admission;
pub mod bo2mp_setup;
pub mod combat_table;
pub mod content_manifest;
pub mod life_front;
pub mod lifecycle;
pub mod loadout;
pub mod local_arm;
pub mod match_apply;
pub mod plugin;
pub mod startup;
pub mod view_subject;

pub use bo2mp_setup::Bo2mpMatchSetup;
pub use content_manifest::{
    AuthorityWeaponProfile, ManifestFact, ManifestGap, RuntimeRuleset, SessionContentManifest,
    SessionManifestError, SessionWeaponId, SessionWeaponManifestRow,
};
pub use life_front::{LifeFrontCensus, LifeNotifyCensus};
pub use lifecycle::{
    LiveWorldIdentity, SessionSwapCompletion, SessionSwapRequest, SessionSwapResult,
    SessionSwapTarget, TeardownGaps, TeardownRequest,
};
pub use loadout::{
    AuthoritativeClassProjection, ClassRow, authoritative_class_lock_reason, perk_catalog_id,
    project_class, resolve_class_weapon,
};
pub use match_apply::{apply_prepared_match, install_script_model_id};
pub use plugin::SessionPlugin;
pub use startup::{PendingConsoleLines, StartupCommands};
