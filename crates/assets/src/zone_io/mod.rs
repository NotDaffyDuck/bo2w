//! Reading the zones of a game into the products a match is built from. Black
//! Ops II is the only game, so there is one lane; the trait is the face the
//! loaders see.

mod census;
mod t6;
mod t6_m2;
mod t6_materials;
mod t6_scripts;
pub mod t6_frontend;

// bo2zm M4: Black Ops II's front end (no map).
pub use t6_m2::{T6Frontend, load_t6_frontend};

use std::collections::BTreeMap;
use std::path::Path;

use asset_core::ZoneTitleKind;
use asset_material::SurfaceShaderRoster;
use asset_model::PooledMeshSkins;
use asset_transport::{LoadCompletion, ZoneTextureData};
use asset_world::{GradeVisionDecodeFault, ScreenGradeLook};

pub use census::{SharedAssetTally, ResidentLevel, ShaderUsageSurvey};

/// What reading zones of one game yields.
pub trait ZoneChannel: Send + Sync {
    /// Reads a map zone, taking from `pooled_skins` and `material_seed` what
    /// the common set already holds.
    fn read_level_geometry(
        &self,
        path: &Path,
        image: &ZoneTextureData,
        progress: &LoadCompletion,
        pooled_skins: PooledMeshSkins,
        material_seed: SurfaceShaderRoster,
        common_film_visions: &mut BTreeMap<String, Result<ScreenGradeLook, GradeVisionDecodeFault>>,
    ) -> ResidentLevel;

    /// Reads the `common_mp` zone every match shares.
    fn read_shared_multiplayer(
        &self,
        path: &Path,
        image: &ZoneTextureData,
        progress: &LoadCompletion,
        decode_colour_maps: bool,
        material_seed: SurfaceShaderRoster,
    ) -> SharedAssetTally;

    /// Reads just the materials, lights, scripts and string tables of a zone.
    fn read_material_census(
        &self,
        path: &Path,
        image: &ZoneTextureData,
        progress: &LoadCompletion,
        material_seed: SurfaceShaderRoster,
    ) -> ShaderUsageSurvey;
}

/// The lane that reads zones of `game`.
pub fn zone_io(game: ZoneTitleKind) -> &'static dyn ZoneChannel {
    match game {
        ZoneTitleKind::T6 => &t6::T6Lane,
    }
}
