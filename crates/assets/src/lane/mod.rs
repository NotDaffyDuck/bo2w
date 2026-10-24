//! Reading the zones of a game into the products a match is built from. Black
//! Ops II is the only game, so there is one lane; the trait is the face the
//! loaders see.

mod census;
mod t6;
mod t6_m2;
mod t6_materials;
mod t6_scripts;
pub mod t6_frontend;

use std::path::Path;

use asset_core::ZoneGame;
use asset_material::MaterialCatalog;
use asset_model::SharedXModelSurfaces;
use asset_transport::{LoadProgress, ZoneImage};

pub use census::{CommonCensus, FilmVisions, LoadedWorld, MaterialPopulation};

/// What reading zones of one game yields.
pub trait ZoneLane: Send + Sync {
    /// Reads a map zone, taking from `shared_surfaces` and `material_seed` what
    /// the common set already holds.
    fn load_world(
        &self,
        path: &Path,
        image: &ZoneImage,
        progress: &LoadProgress,
        shared_surfaces: SharedXModelSurfaces,
        material_seed: MaterialCatalog,
        common_film_visions: &mut FilmVisions,
    ) -> LoadedWorld;

    /// Reads the `common_mp` zone every match shares.
    fn load_common_mp(
        &self,
        path: &Path,
        image: &ZoneImage,
        progress: &LoadProgress,
        decode_color_maps: bool,
        material_seed: MaterialCatalog,
    ) -> CommonCensus;

    /// Reads just the materials, lights, scripts and string tables of a zone.
    fn load_material_population(
        &self,
        path: &Path,
        image: &ZoneImage,
        progress: &LoadProgress,
        material_seed: MaterialCatalog,
    ) -> MaterialPopulation;
}

/// The lane that reads zones of `game`.
pub fn lane(game: ZoneGame) -> &'static dyn ZoneLane {
    match game {
        ZoneGame::T6 => &t6::T6Lane,
    }
}
