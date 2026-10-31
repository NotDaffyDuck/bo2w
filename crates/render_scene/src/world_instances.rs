//! The components a spawned world object carries.

use bevy::prelude::*;

/// A model the map's script placed: its identity, which model it shows now,
/// where it stands, and the script-visible state its skeleton is built from.
#[derive(Component, Clone, Debug)]
pub struct LevelScriptModelInstance {
    pub id: asset_world::ScriptModelId,
    pub server_owned_model_owner: Option<sim::ServerModelSource>,
    pub active_mesh: asset_world::LevelModelAssetId,
    pub place: Transform,
    pub illum_origin: [f32; 3],
    pub puppet_state: xmodel_runtime::RigPoseRecord,
    pub metadata: asset_world::ScriptMeshDescriptor,
    pub game_actor_number: Option<u16>,
}

/// A dynamic entity the map places (a breakable, a physics prop): its current
/// model, health, and what happens when it dies.
#[derive(Component, Clone, Debug)]
pub struct LevelDebrisInstance {
    pub index: u16,
    pub ty: asset_world::MovableObjectKind,
    pub active_mesh: asset_world::LevelModelAssetId,
    pub place: Transform,
    pub illum_origin: [f32; 3],
    pub physics_template: Option<asset_world::SelfContainedPhysicsPreset>,
    pub hp: i32,
    pub destroy_effect: Option<String>,
    pub gone: bool,
}
