//! What a lane hands back after reading a zone.

use std::collections::BTreeMap;

use crate::{
    ScriptSources,
    load_stages::{StagedLevel, LevelRenderRules},
};
use anim_bank::XAnimBuild;
use asset_game::{
    SnapshotStringTable, EffectRoster, EffectMeshRoster, SelfContainedImpactFxTable, TracerRoster, WeaponBuild,
};
use asset_material::SurfaceShaderRoster;
use asset_model::{
    BodyMeshBuild, FpvMeshBuild, ProjectileMeshBuild, PooledMeshSkins, WorldWeaponBuild,
};
use asset_world::{SnapshotLightDef, ScreenGradeLook, GradeVisionDecodeFault, LevelModelPlacementRoster};
use weapon_t6::{hitloc::HITLOC_COUNT, penetration::BulletPierceDepthChart};

/// Everything reading one map zone produced.
#[derive(Default)]
pub struct ResidentLevel {
    pub scripts: ScriptSources,
    pub world: StagedLevel,
    /// The materials the map zone itself defined. Material numbers in `world`
    /// index this pool until the match publishes the one for everything.
    pub mat_list: SurfaceShaderRoster,
    pub collision: Option<asset_world::CollisionClipData>,
    pub spawns: Vec<asset_world::RespawnMarker>,
    pub bodies: BodyMeshBuild,
    pub first_person_meshes: FpvMeshBuild,
    pub animation_refs: XAnimBuild,
    /// Weapons the map zone defines itself, and their third-person models.
    pub weapons: WeaponBuild,
    pub level_placed_weapons: WorldWeaponBuild,
    pub tracers: TracerRoster,
    /// Penetration depths and hit-location damage the map's zones carry, when
    /// the common set did not.
    pub pen_table: Option<BulletPierceDepthChart>,
    pub lochit_table: Option<[f32; HITLOC_COUNT]>,
    pub facts: crate::MapFacts,
    pub sound: Option<Result<asset_audio::AudioRoster, String>>,
    pub load_log: Vec<String>,
}

impl ResidentLevel {
    /// A world with nothing in it but the reason why.
    pub fn with_gap(policy: LevelRenderRules, reason: impl Into<String>) -> Self {
        Self {
            world: StagedLevel {
                policy,
                ..Default::default()
            },
            load_log: vec![reason.into()],
            ..Default::default()
        }
    }
}

/// Everything reading the shared `common_mp` zone produced.
#[derive(Default)]
pub struct SharedAssetTally {
    pub scripts: ScriptSources,
    pub stage_models: LevelModelPlacementRoster,
    pub pooled_skins: PooledMeshSkins,
    pub weapons: WeaponBuild,
    pub cac_tables: Vec<SnapshotStringTable>,
    pub fpv: FpvMeshBuild,
    pub level_placed_weapons: WorldWeaponBuild,
    pub shell_visual_models: ProjectileMeshBuild,
    pub animation_refs: XAnimBuild,
    pub fx: EffectRoster,
    pub effect_meshes: EffectMeshRoster,
    pub tracers: TracerRoster,
    pub impact_effect: Option<SelfContainedImpactFxTable>,
    pub material_population: SurfaceShaderRoster,
    pub light_defs: Vec<SnapshotLightDef>,
    pub load_log: Vec<String>,
    pub pen_table: BulletPierceDepthChart,
    pub wall_pierce_table_ready: bool,
    pub lochit_table: Option<[f32; HITLOC_COUNT]>,
    pub model_traversal: crate::ReadiedXModelWalkCensus,
    pub vision_sets: BTreeMap<String, Result<ScreenGradeLook, GradeVisionDecodeFault>>,
}

/// What walking one zone's materials produced.
#[derive(Default)]
pub struct ShaderUsageSurvey {
    pub mat_list: SurfaceShaderRoster,
    pub light_defs: Vec<SnapshotLightDef>,
    pub load_log: Vec<String>,
    pub cac_tables: Vec<SnapshotStringTable>,
    pub scripts: ScriptSources,
}
