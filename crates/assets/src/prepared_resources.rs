//! The catalogs a prepared match hands the session, each installed as a
//! resource of its own and retired again when the match ends.

use std::sync::Arc;

use bevy::prelude::Resource;

use anim_bank::MotionClipRoster;
use asset_core::{AssetDependencyLink, MapXModelRange, XAnimRange};
use asset_game::{TranslationStringIndex, WeaponLedger};
use asset_model::{CharacterMeshRoster, FirstPersonMeshRoster, MissileMeshRoster, LevelWeaponRoster};

#[derive(Clone, Debug, Default, Resource)]
pub struct StagedWeapons(pub Arc<WeaponLedger>);

/// The sound names the match's effects reference through their type-10 entries.
#[derive(Clone, Debug, Default, Resource)]
pub struct RoundTenthKindAudioClues(pub Vec<String>);

#[derive(Clone, Debug, Default, Resource)]
pub struct ReadiedFpvMeshes(pub FirstPersonMeshRoster);

#[derive(Clone, Debug, Default, Resource)]
pub struct ReadiedCharacterBodies(pub Arc<CharacterMeshRoster>);

#[derive(Clone, Debug, Default, Resource)]
pub struct ReadiedWorldWeapons(pub LevelWeaponRoster);

#[derive(Clone, Debug, Default, Resource)]
pub struct ReadiedProjectileMeshes(pub MissileMeshRoster);

#[derive(Clone, Debug, Default, Resource)]
pub struct StagedXAnims(pub MotionClipRoster);

#[derive(Clone, Debug, Default, Resource)]
pub struct ReadiedLocalText(pub TranslationStringIndex);

/// A marker the session installs for the model census; nothing fills it in.
#[derive(Clone, Copy, Debug, Default, Resource)]
pub struct ReadiedXModelWalkCensus;

/// What a destructible plays and leaves behind when it dies, by kind. The
/// hints name what to look for; `clip` and `husk` are what was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BreakableDemiseRecord {
    pub demise_kind: &'static str,
    pub collision_clue: &'static str,
    pub shell_corpse_clue: &'static str,
    pub clip: AssetDependencyLink<XAnimRange>,
    pub husk: AssetDependencyLink<MapXModelRange>,
}

#[derive(Clone, Debug, Default, Resource)]
pub struct ReadiedBreakableDeath(pub Vec<BreakableDemiseRecord>);
