//! Getting content off disk: where the games are, the zone files and IWD
//! archives in them, a cache for prepared blobs, and progress reporting for
//! the load that uses them.

pub mod blob_cache;
pub mod install_scan;
pub mod sound_pack;
pub mod sound_roots;
pub mod progress;
pub mod t6_extra;
pub mod zone;

pub use blob_cache::{memo_fetch, memo_store, hash_text_64, fnv1a64_extend};
pub use asset_core::ZoneTitleKind;
pub use install_scan::{
    GameInstallDir, MapPack, ZoneArchive, prepare_output_folder, locate_shared_mp_zone,
    locate_runtime_zone, locate_zone_file, locate_tree_zone, install_root_of_zone,
    game_dir_from_environment, install_folders_summary, list_mp_map_packs, multiplayer_map_names, read_env_file,
    loading_caption, zone_key_parts, title_of_zone_path,
};
pub use sound_pack::{SoundArchiveIndex, main_dir_for_zone, read_sound_pack_member};
pub use sound_roots::{CategorySoundArchive, CategoryTree, CategoryTreeSet};
pub use progress::{
    LoadLaneTiming, LoadCompletion, LoadSnapshot, StageEnd, StageHandle, StageId, StageKey,
    StageOutcome, StageScope, StageSnapshot, WorkCount, high_water_memory,
    working_set_size,
};
pub use zone::{T6ZoneMemory, ZoneTextureData, ZoneUnpackFault, mount_zone, open_shared_zone};
