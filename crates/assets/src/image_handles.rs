//! The GPU images a loaded map's materials and lighting draw with.

use std::ops::Deref;
use std::sync::Arc;

use bevy::prelude::*;

use render_material::MaterialGenerationId;

/// The lightmap images of one lightmap slot. The diagnostic images stand in
/// when the map's own lighting is switched off.
#[derive(Clone, Debug)]
pub struct RuntimeLightmapHandles {
    pub primary: Option<Handle<Image>>,
    pub secondary: Option<Handle<Image>>,
    pub secondary_b: Option<Handle<Image>>,
    pub ambient_diagnostic: Handle<Image>,
    pub directional_diagnostic: Handle<Image>,
    pub sun_mask_diagnostic: Handle<Image>,
}

/// Everything the handles hold, tagged with the material generation they
/// were made for. Slots line up with the map's own tables; a slot is empty
/// when its image never loaded.
#[derive(Clone, Debug, Default)]
pub struct RuntimeImageHandlesData {
    pub generation_id: MaterialGenerationId,
    pub material_images: Vec<Option<Handle<Image>>>,
    pub material_names: Vec<String>,
    pub reflection_probes: Vec<Option<Handle<Image>>>,
    pub lightmaps: Vec<Option<RuntimeLightmapHandles>>,
    pub model_lighting: Option<Handle<Image>>,
}

/// The loaded map's image handles, shared by reference: cloning is cheap and
/// [`ptr_eq`](Self::ptr_eq) says whether two clones are the same set.
#[derive(Resource, Clone, Debug, Default)]
pub struct RuntimeImageHandles(Arc<RuntimeImageHandlesData>);

impl Deref for RuntimeImageHandles {
    type Target = RuntimeImageHandlesData;

    fn deref(&self) -> &RuntimeImageHandlesData {
        &self.0
    }
}

impl RuntimeImageHandles {
    pub fn from_pools(
        generation_id: MaterialGenerationId,
        material_images: Vec<Option<Handle<Image>>>,
        material_names: Vec<String>,
        reflection_probes: Vec<Option<Handle<Image>>>,
        lightmaps: Vec<Option<RuntimeLightmapHandles>>,
        model_lighting: Option<Handle<Image>>,
    ) -> Self {
        Self(Arc::new(RuntimeImageHandlesData {
            generation_id,
            material_images,
            material_names,
            reflection_probes,
            lightmaps,
            model_lighting,
        }))
    }

    /// Whether `other` is this very set, not just an equal one.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_are_the_same_set_and_a_rebuild_is_not() {
        let handles = RuntimeImageHandles::default();
        assert!(handles.ptr_eq(&handles.clone()));
        assert!(!handles.ptr_eq(&RuntimeImageHandles::default()));
    }

    #[test]
    fn the_pools_come_back_through_the_deref() {
        let handles = RuntimeImageHandles::from_pools(
            MaterialGenerationId::default(),
            vec![None, None],
            vec!["a".into(), "b".into()],
            Vec::new(),
            Vec::new(),
            None,
        );
        assert_eq!(handles.material_images.len(), 2);
        assert_eq!(handles.material_names[1], "b");
        assert!(handles.model_lighting.is_none());
    }
}
