//! Reading the zones a match names and putting what they hold together as one
//! prepared match.

mod common_set;
mod pool;
mod prepare;
mod products;
mod resident;
mod startup;

// What the parts below share through `use super::*`.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::lane::{LoadedWorld, lane};
use asset_anim::XAnimBuild;
use asset_game::{
    FxCatalog, LocalizeCatalog, MP_LOCALIZED_ZONES, WeaponBuild, load_localize_catalog_in_lane,
};
use asset_material::MaterialCatalog;
use asset_model::{BodyMeshCatalog, FpvMeshBuild, ProjectileMeshBuild, WorldWeaponBuild};
use asset_transport::{
    LoadProgress, StageHandle, StageId, find_runtime_zone, find_zone_for_tree, games_root_from_env,
    open_zone_shared,
};
use common_set::*;
use prepare::*;
use startup::*;

pub use asset_world::WorldDrawPolicy;
pub use pool::{load_pool, load_workers, publish_process_cpus};
pub use products::{MatchLoadOutcome, PreparedMatch, PreparedWorld};
pub use resident::{load_prepared_match, note_gpu_resident, release_resident_cpu_images};
