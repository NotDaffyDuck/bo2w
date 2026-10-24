//! What the presenter shares between the systems that draw the world.

use std::sync::Arc;

use bevy::prelude::*;
use render_material::{PreparedMaterialTable, RuntimeMaterialCatalog};

/// The material catalog, its prepared draw table, and one loaded image per
/// texture the catalog names (`None` while a texture is missing).
#[derive(Resource, Clone, Debug, Default)]
pub struct TessMaterials {
    pub catalog: Arc<RuntimeMaterialCatalog>,
    pub prepared: Arc<PreparedMaterialTable>,
    pub material_images: Arc<Vec<Option<Handle<Image>>>>,
}

/// Whether the world's static geometry has been spawned yet.
#[derive(Resource, Clone, Debug, Default)]
pub struct WorldPresentFacts {
    pub spawned: bool,
}
