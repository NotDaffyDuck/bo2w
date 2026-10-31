//! Reading the zones a match names and putting what they hold together as one
//! prepared match.

mod common_set;
mod load_threads;
mod prepare;
mod stage_outputs;
mod resident;
mod startup;

// What the parts below share through `use super::*`.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::zone_io::{ResidentLevel, zone_io};
use anim_bank::XAnimBuild;
use asset_game::{
    EffectRoster, TranslationStringIndex, VERSUS_LANGUAGE_ZONES, WeaponBuild, load_translation_lane,
};
use asset_material::SurfaceShaderRoster;
use asset_model::{CharacterMeshRoster, FpvMeshBuild, ProjectileMeshBuild, WorldWeaponBuild};
use asset_transport::{
    LoadCompletion, StageHandle, StageId, locate_runtime_zone, locate_tree_zone, game_dir_from_environment,
    open_shared_zone,
};
use common_set::*;
use prepare::*;
use startup::*;

pub use asset_world::LevelRenderRules;
pub use load_threads::{loader_pool, spawn_reader_threads, announce_worker_cores};
pub use stage_outputs::{MatchLoadOutcome, ReadiedSession, StagedLevel};
pub use resident::{read_staged_round, note_gpu_resident, release_resident_cpu_images};
