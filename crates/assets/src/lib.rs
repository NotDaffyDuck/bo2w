//! Loading what a match needs from the game's files, and handing the prepared
//! products to the rest of the engine.

mod lane;
mod load_overlay;
mod load_process;
mod match_load_admit;
mod match_load_state;
mod match_load_walk;
mod prepared_map;
mod prepared_resources;
mod script_bundle;
mod script_names;
mod script_set;
mod t6_hud_assets;
mod teardown;

pub mod image_handles;
pub mod plugin;
pub mod session_load;

pub use asset_core::{
    AssetEdge, FpvMeshIndex, MaterialIndex, MaterialSpace, TracerSpace, WorldWeaponIndex,
};
pub use asset_transport::{
    NamespaceSoundIwd, NamespaceTrees, cache_get, cache_put, fnv1a64, fnv1a64_more,
};
pub use lane::t6_frontend::{T6Frontend, load_t6_frontend, plain_rgba8};
pub use lighting_t6::MODEL_LIGHTING_TILE_BYTES;
pub use load_overlay::{LoadingPreviewSource, LoadingScreen};
pub use load_process::MapLoadProcess;
pub use match_load_state::{
    MapLoadApproval, MatchLoadAbort, MatchLoadAccepted, MatchLoadBusy, MatchLoadDispatch,
    MatchLoadRequest, PreparedMatchReady, PreparedMatchSound,
};
pub use plugin::AssetPlugin;
pub use prepared_map::{MapFacts, MatchMaterials, PreparedMap};
pub use prepared_resources::{
    DestructibleDeathRow, MatchType10SoundHints, PreparedBodies, PreparedDestructibleDeath,
    PreparedFpvMeshes, PreparedLocalizedStrings, PreparedProjectileMeshes, PreparedWeapons,
    PreparedWorldWeapons, PreparedXAnims, PreparedXModelWalkCensus,
};
pub use script_bundle::ScriptSources;
pub use script_set::{T6AnimFacts, T6ScriptSet};
pub use session_load::{
    MatchLoadOutcome, PreparedMatch, PreparedWorld, load_pool, load_prepared_match, load_workers,
    note_gpu_resident, publish_process_cpus, release_resident_cpu_images,
};
pub use t6_hud_assets::{T6HudFont, T6HudFonts, T6HudIcons, T6Ui};
