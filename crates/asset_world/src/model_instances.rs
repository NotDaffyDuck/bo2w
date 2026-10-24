//! Where models stand in a loaded map: the static ones the map places, and
//! the script-driven ones with the fields their scripts set.

use bevy::prelude::Transform;

use crate::model_scene_catalog::MapXModelAssetKey;

/// One static model standing in the map.
#[derive(Clone, Copy, Debug)]
pub struct StaticModelPlacement {
    /// Index of the model's mesh.
    pub mesh: usize,
    pub transform: Transform,
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub scale: f32,
    pub cull_dist: u16,
    pub reflection_probe_index: u8,
    pub primary_light_index: u8,
    pub flags: u8,

    /// T6: the light the fallback draw uses. RGB is sqrt(light / 32) from the
    /// light grid at the model's origin; alpha 255 means its mesh carries baked
    /// vertex light instead. `None` for any other world.
    pub packed_lighting: Option<[u8; 4]>,
    /// T6: the model's primary light (an index into the map's primary lights)
    /// and how visible it is there (0..255, from the light grid). `[0, 0]` for
    /// any other world.
    pub t6_primary: [u8; 2],
    /// T6: the model's own light, from its light-grid coefficient set
    /// (`colorsIndex`). One more than its index in `t6_model_lights`; 0 for none.
    pub t6_light: u32,
}

/// The fields a script model carries from its map entity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptModelMetadata {
    pub gameobject: String,
    pub t5_destructible: Option<std::sync::Arc<xmodel_runtime::T5DestructibleDef>>,
    pub brush_link: crate::ScriptBrushModelLink,
}

/// A script model standing in the map.
#[derive(Clone, Debug)]
pub struct ScriptModelSceneInstance {
    pub id: crate::ScriptModelId,
    /// The model it currently shows.
    pub current_model: MapXModelAssetKey,
    pub transform: Transform,
    pub lighting_origin: [f32; 3],
    pub dobj_state: xmodel_runtime::DObjSemanticState,
    pub metadata: ScriptModelMetadata,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_script_model_carries_nothing() {
        let metadata = ScriptModelMetadata::default();
        assert!(metadata.gameobject.is_empty());
        assert!(metadata.t5_destructible.is_none());
        assert_eq!(metadata, ScriptModelMetadata::default());
    }
}
