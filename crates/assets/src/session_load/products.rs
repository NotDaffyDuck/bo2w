//! What a finished match preparation holds.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{PreparedMap, ScriptSources};
use asset_anim::XAnimCatalog;
use asset_game::{LocalizeCatalog, WeaponRegistry};
use asset_model::{BodyMeshCatalog, FpvMeshCatalog, WorldWeaponCatalog};
use asset_world::{
    ClipCollision, FilmVision, FilmVisionParseError, IntermissionView, WorldDraw, WorldDrawPolicy,
};

/// The map's own world: its drawn geometry, placed models, lighting and
/// scripted fixtures.
#[derive(Default, Clone)]
pub struct PreparedWorld {
    pub draw: Option<WorldDraw>,
    pub dynamic_light: Option<asset_world::ResolvedLightDef>,
    pub static_model_meshes: Vec<asset_world::ModelMesh>,
    pub static_model_instances: Vec<Option<asset_world::StaticModelPlacement>>,
    pub map_xmodel_scene_assets: asset_world::MapXModelSceneCatalog,
    pub script_model_instances: Vec<asset_world::ScriptModelSceneInstance>,
    pub script_brush_models: Vec<asset_world::ScriptBrushModelPlacement>,
    pub dyn_ents: asset_world::DynEntCatalog,
    pub smodel_lighting_samples: Vec<asset_model::SmodelLightingSample>,
    pub light_grid: Option<asset_model::OwnedLightGrid>,
    pub fx: asset_game::FxCatalog,
    pub fx_models: asset_game::FxModelCatalog,
    pub t6_glass: Option<asset_world::T6Glass>,
    pub impact_fx: Option<asset_game::OwnedFxImpactTable>,
    pub reflection_probe_images: Vec<Option<bevy::prelude::Image>>,
    pub intermission_view: Option<IntermissionView>,
    pub exp_fog: Option<asset_world::ExpFog>,
    pub film_vision: Option<FilmVision>,
    pub film_visions: BTreeMap<String, Result<FilmVision, FilmVisionParseError>>,
    pub createart_name: Option<String>,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub world_bounds: Option<[f32; 6]>,
    pub policy: WorldDrawPolicy,
}

/// Everything one match needs, ready to hand to the game.
#[derive(Default, Clone)]
pub struct PreparedMatch {
    pub scripts: ScriptSources,
    pub world: PreparedWorld,
    pub fx: asset_game::FxDefinitions,
    pub materials: crate::MatchMaterials,
    pub clip: Option<Arc<ClipCollision>>,
    pub weapons: Arc<WeaponRegistry>,
    pub fpv_meshes: FpvMeshCatalog,
    pub bodies: Arc<BodyMeshCatalog>,
    pub world_weapons: WorldWeaponCatalog,
    pub projectile_meshes: asset_model::ProjectileMeshCatalog,
    pub xanims: XAnimCatalog,
    pub destructible_death: Vec<crate::DestructibleDeathRow>,
    pub tracers: asset_game::TracerDefinitions,
    pub strings: LocalizeCatalog,
    /// Log lines about how the preparation went; nothing reads them back.
    pub report: Vec<String>,
    pub prepared_map: PreparedMap,
    pub pen_table: weapon_t6::penetration::PenetrationDepthTable,
    pub pen_table_loaded: bool,
    pub lochit_table: Option<[f32; weapon_t6::hitloc::HITLOC_COUNT]>,
    pub xmodel_walk: crate::PreparedXModelWalkCensus,
    pub sound: Option<Result<asset_audio::SoundCatalog, String>>,
}

pub enum MatchLoadOutcome {
    Ready(PreparedMatch),
    Canceled,
}
