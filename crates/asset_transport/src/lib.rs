//! Getting content off disk: where the games are, the zone files and IWD
//! archives in them, a cache for prepared blobs, and progress reporting for
//! the load that uses them.

pub mod artifact_cache;
pub mod discover;
pub mod iwd;
pub mod load_jobs;
pub mod namespace_trees;
pub mod progress;
pub mod zone;

pub use artifact_cache::{cache_get, cache_put, fnv1a64, fnv1a64_more};
pub use asset_core::ZoneGame;
pub use discover::{
    GamesRoot, MapPack, ZoneFile, ensure_artifacts_dir, find_runtime_common_mp,
    find_runtime_zone, find_zone_file, find_zone_for_tree, game_root_for_zone,
    games_root_from_env, games_root_report, list_mp_map_packs, list_mp_maps, load_dotenv,
    map_load_title, split_zone_key, zone_game_for_path,
};
pub use iwd::{IwdSoundIndex, game_main_for_zone, read_iwd_named};
pub use load_jobs::{CacheResult, JobKind};
pub use namespace_trees::{NamespaceSoundIwd, NamespaceTree, NamespaceTrees};
pub use progress::{
    LoadLaneTiming, LoadProgress, LoadSnapshot, StageEnd, StageHandle, StageId, StageKey,
    StageOutcome, StageScope, StageSnapshot, WorkCount, peak_resident_bytes,
    process_resident_bytes,
};
pub use zone::{T6ZoneMemory, ZoneImage, ZoneOpenError, open_zone, open_zone_shared};
