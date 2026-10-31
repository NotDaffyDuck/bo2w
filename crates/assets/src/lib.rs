//! Loading what a match needs from the game's files, and handing the prepared
//! products to the rest of the engine.

mod zone_io;
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
mod session_teardown;

pub mod surf_images;
pub mod load_wiring;
pub mod load_stages;

pub use asset_core::{
    AssetDependencyLink, FpvMeshSlot, SurfaceMaterialId, SurfaceShaderSpace, TracerRange, WorldWeaponSlot,
};
pub use asset_transport::{
    CategorySoundArchive, CategoryTreeSet, memo_fetch, memo_store, hash_text_64, fnv1a64_extend,
};
pub use zone_io::t6_frontend::{T6Frontend, load_t6_frontend, plain_rgba8};
pub use lighting_t6::ILLUM_TILE_LEN;
pub use load_overlay::{LoadBackdropOrigin, LoadBackdrop};
pub use load_process::MapLoadProcess;
pub use match_load_state::{
    MapLoadApproval, SessionLoadCancel, SessionLoadApproved, SessionLoadOccupied, MatchLoadDispatch,
    SessionLoadOrder, ReadiedSessionSignal, PreparedMatchSound,
};
pub use load_wiring::ZoneAssetPlugin;
pub use prepared_map::{MapFacts, MatchMaterials, ReadiedMapData};
pub use prepared_resources::{
    BreakableDemiseRecord, RoundTenthKindAudioClues, ReadiedCharacterBodies, ReadiedBreakableDeath,
    ReadiedFpvMeshes, ReadiedLocalText, ReadiedProjectileMeshes, StagedWeapons,
    ReadiedWorldWeapons, StagedXAnims, ReadiedXModelWalkCensus,
};
pub use script_bundle::ScriptSources;
pub use script_set::{T6AnimFacts, T6ScriptSet};
pub use load_stages::{
    MatchLoadOutcome, ReadiedSession, StagedLevel, loader_pool, read_staged_round, spawn_reader_threads,
    note_gpu_resident, announce_worker_cores, release_resident_cpu_images,
};
pub use t6_hud_assets::{
    T6HudFont, T6HudFonts, T6HudIcons, T6ScreenFilter, T6ScreenFilters, T6Ui,
};
