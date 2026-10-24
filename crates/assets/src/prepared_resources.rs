//! The catalogs a prepared match hands the session, each installed as a
//! resource of its own and retired again when the match ends.

use std::sync::Arc;

use bevy::prelude::Resource;

use asset_anim::XAnimCatalog;
use asset_core::{AssetEdge, MapXModelSpace, XAnimSpace};
use asset_game::{LocalizeCatalog, WeaponRegistry};
use asset_model::{BodyMeshCatalog, FpvMeshCatalog, ProjectileMeshCatalog, WorldWeaponCatalog};

#[derive(Clone, Debug, Default, Resource)]
pub struct PreparedWeapons(pub Arc<WeaponRegistry>);

/// The sound names the match's effects reference through their type-10 entries.
#[derive(Clone, Debug, Default, Resource)]
pub struct MatchType10SoundHints(pub Vec<String>);

#[derive(Clone, Debug, Default, Resource)]
pub struct PreparedFpvMeshes(pub FpvMeshCatalog);

#[derive(Clone, Debug, Default, Resource)]
pub struct PreparedBodies(pub Arc<BodyMeshCatalog>);

#[derive(Clone, Debug, Default, Resource)]
pub struct PreparedWorldWeapons(pub WorldWeaponCatalog);

#[derive(Clone, Debug, Default, Resource)]
pub struct PreparedProjectileMeshes(pub ProjectileMeshCatalog);

#[derive(Clone, Debug, Default, Resource)]
pub struct PreparedXAnims(pub XAnimCatalog);

#[derive(Clone, Debug, Default, Resource)]
pub struct PreparedLocalizedStrings(pub LocalizeCatalog);

/// A marker the session installs for the model census; nothing fills it in.
#[derive(Clone, Copy, Debug, Default, Resource)]
pub struct PreparedXModelWalkCensus;

/// What a destructible plays and leaves behind when it dies, by kind. The
/// hints name what to look for; `clip` and `husk` are what was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DestructibleDeathRow {
    pub kind: &'static str,
    pub clip_hint: &'static str,
    pub husk_hint: &'static str,
    pub clip: AssetEdge<XAnimSpace>,
    pub husk: AssetEdge<MapXModelSpace>,
}

#[derive(Clone, Debug, Default, Resource)]
pub struct PreparedDestructibleDeath(pub Vec<DestructibleDeathRow>);
