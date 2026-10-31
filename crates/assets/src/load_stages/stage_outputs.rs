//! What a finished match preparation holds.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{ReadiedMapData, ScriptSources};
use anim_bank::MotionClipRoster;
use asset_game::{TranslationStringIndex, WeaponLedger};
use asset_model::{CharacterMeshRoster, FirstPersonMeshRoster, LevelWeaponRoster};
use asset_world::{
    CollisionClipData, ScreenGradeLook, GradeVisionDecodeFault, BetweenRoundsCamera, LevelDrawCall, LevelRenderRules,
};

/// The map's own world: its drawn geometry, placed models, lighting and
/// scripted fixtures.
#[derive(Default, Clone)]
pub struct StagedLevel {
    pub draw: Option<LevelDrawCall>,
    pub dynamic_light: Option<asset_world::ResolvedLightDef>,
    pub prop_meshes: Vec<asset_world::StaticMeshData>,
    pub prop_instances: Vec<Option<asset_world::FixedPropPlacement>>,
    pub map_figure_stage_assets: asset_world::LevelModelPlacementRoster,
    pub scripted_prop_placements: Vec<asset_world::ScriptedPropPlacement>,
    pub script_solid_models: Vec<asset_world::ScriptSolidPlacementInfo>,
    pub debris_pieces: asset_world::MovableObjectRoster,
    pub prop_illum_samples: Vec<asset_model::StaticPropLightSample>,
    pub light_grid: Option<asset_model::DetachedLightGrid>,
    pub fx: asset_game::EffectRoster,
    pub effect_meshes: asset_game::EffectMeshRoster,
    pub t6_glass: Option<asset_world::T6Glass>,
    pub impact_effect: Option<asset_game::SelfContainedImpactFxTable>,
    pub env_probe_images: Vec<Option<bevy::prelude::Image>>,
    pub scoreboard_eye: Option<BetweenRoundsCamera>,
    pub exp_haze: Option<asset_world::ExpHaze>,
    pub colour_grade: Option<ScreenGradeLook>,
    pub vision_sets: BTreeMap<String, Result<ScreenGradeLook, GradeVisionDecodeFault>>,
    pub art_profile_name: Option<String>,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub level_bounds: Option<[f32; 6]>,
    pub policy: LevelRenderRules,
}

/// Everything one match needs, ready to hand to the game.
#[derive(Default, Clone)]
pub struct ReadiedSession {
    pub scripts: ScriptSources,
    pub world: StagedLevel,
    pub fx: asset_game::FxDefinitions,
    pub materials: crate::MatchMaterials,
    pub clip: Option<Arc<CollisionClipData>>,
    pub weapons: Arc<WeaponLedger>,
    pub first_person_meshes: FirstPersonMeshRoster,
    pub bodies: Arc<CharacterMeshRoster>,
    pub level_placed_weapons: LevelWeaponRoster,
    pub shell_visual_models: asset_model::MissileMeshRoster,
    pub animation_refs: MotionClipRoster,
    pub destructible_death: Vec<crate::BreakableDemiseRecord>,
    pub tracers: asset_game::TracerDefinitions,
    pub strings: TranslationStringIndex,
    /// Log lines about how the preparation went; nothing reads them back.
    pub report: Vec<String>,
    pub prepared_map: ReadiedMapData,
    pub pen_table: weapon_t6::penetration::BulletPierceDepthChart,
    pub wall_pierce_table_ready: bool,
    pub lochit_table: Option<[f32; weapon_t6::hitloc::HITLOC_COUNT]>,
    pub model_traversal: crate::ReadiedXModelWalkCensus,
    pub sound: Option<Result<asset_audio::AudioRoster, String>>,
}

pub enum MatchLoadOutcome {
    Ready(ReadiedSession),
    Canceled,
}
