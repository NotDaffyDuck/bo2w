//! What the presenter shares between the systems that draw the world.

use std::sync::Arc;

use bevy::prelude::*;
use render_material::LiveMaterialLedger;
use render_material::StagedMaterialTable;

/// The material catalog, its prepared draw table, and one loaded image per
/// texture the catalog names (`None` while a texture is missing).
#[derive(Resource, Clone, Debug, Default)]
pub struct StitchMaterials {
    pub catalog: Arc<LiveMaterialLedger>,
    pub prepared: Arc<StagedMaterialTable>,
    pub shader_textures: Arc<Vec<Option<Handle<Image>>>>,
}

/// Whether the world's static geometry has been spawned yet.
#[derive(Resource, Clone, Debug, Default)]
pub struct LevelPresentFacts {
    pub spawned: bool,
}
