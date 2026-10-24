//! What a lane hands back after reading a zone.

use std::collections::BTreeMap;

use crate::{
    ScriptSources,
    session_load::{PreparedWorld, WorldDrawPolicy},
};
use asset_anim::XAnimBuild;
use asset_game::{
    CapturedStringTable, FxCatalog, FxModelCatalog, OwnedFxImpactTable, TracerCatalog, WeaponBuild,
};
use asset_material::MaterialCatalog;
use asset_model::{
    BodyMeshBuild, FpvMeshBuild, ProjectileMeshBuild, SharedXModelSurfaces, WorldWeaponBuild,
};
use asset_world::{CapturedLightDef, FilmVision, FilmVisionParseError, MapXModelSceneCatalog};
use weapon_t6::{hitloc::HITLOC_COUNT, penetration::PenetrationDepthTable};

/// Film visions by name; one that failed to parse keeps its error.
pub type FilmVisions = BTreeMap<String, Result<FilmVision, FilmVisionParseError>>;

/// Everything reading one map zone produced.
#[derive(Default)]
pub struct LoadedWorld {
    pub scripts: ScriptSources,
    pub world: PreparedWorld,
    /// The materials the map zone itself defined. Material numbers in `world`
    /// index this pool until the match publishes the one for everything.
    pub materials: MaterialCatalog,
    pub collision: Option<asset_world::ClipCollision>,
    pub spawns: Vec<asset_world::SpawnPoint>,
    pub bodies: BodyMeshBuild,
    pub fpv_meshes: FpvMeshBuild,
    pub xanims: XAnimBuild,
    /// Weapons the map zone defines itself, and their third-person models.
    pub weapons: WeaponBuild,
    pub world_weapons: WorldWeaponBuild,
    pub tracers: TracerCatalog,
    /// Penetration depths and hit-location damage the map's zones carry, when
    /// the common set did not.
    pub pen_table: Option<PenetrationDepthTable>,
    pub lochit_table: Option<[f32; HITLOC_COUNT]>,
    pub facts: crate::MapFacts,
    pub sound: Option<Result<asset_audio::SoundCatalog, String>>,
    pub report: Vec<String>,
}

impl LoadedWorld {
    /// A world with nothing in it but the reason why.
    pub fn with_gap(policy: WorldDrawPolicy, reason: impl Into<String>) -> Self {
        Self {
            world: PreparedWorld {
                policy,
                ..Default::default()
            },
            report: vec![reason.into()],
            ..Default::default()
        }
    }
}

/// Everything reading the shared `common_mp` zone produced.
#[derive(Default)]
pub struct CommonCensus {
    pub scripts: ScriptSources,
    pub scene_models: MapXModelSceneCatalog,
    pub shared_surfaces: SharedXModelSurfaces,
    pub weapons: WeaponBuild,
    pub cac_tables: Vec<CapturedStringTable>,
    pub fpv: FpvMeshBuild,
    pub world_weapons: WorldWeaponBuild,
    pub projectile_meshes: ProjectileMeshBuild,
    pub xanims: XAnimBuild,
    pub fx: FxCatalog,
    pub fx_models: FxModelCatalog,
    pub tracers: TracerCatalog,
    pub impact_fx: Option<OwnedFxImpactTable>,
    pub material_population: MaterialCatalog,
    pub light_defs: Vec<CapturedLightDef>,
    pub report: Vec<String>,
    pub pen_table: PenetrationDepthTable,
    pub pen_table_loaded: bool,
    pub lochit_table: Option<[f32; HITLOC_COUNT]>,
    pub xmodel_walk: crate::PreparedXModelWalkCensus,
    pub film_visions: FilmVisions,
}

/// What walking one zone's materials produced.
#[derive(Default)]
pub struct MaterialPopulation {
    pub materials: MaterialCatalog,
    pub light_defs: Vec<CapturedLightDef>,
    pub report: Vec<String>,
    pub cac_tables: Vec<CapturedStringTable>,
    pub scripts: ScriptSources,
}
