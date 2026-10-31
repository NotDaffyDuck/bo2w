//! The GPU images a loaded map's materials and lighting draw with.

use std::ops::Deref;
use std::sync::Arc;

use bevy::prelude::*;

use render_material::MaterialEpochId;

/// The lightmap images of one lightmap slot. The diagnostic images stand in
/// when the map's own lighting is switched off.
#[derive(Clone, Debug)]
pub struct LiveLightbakeHandles {
    pub primary: Option<Handle<Image>>,
    pub secondary: Option<Handle<Image>>,
    pub auxiliary_second: Option<Handle<Image>>,
    pub background_probe_note: Handle<Image>,
    pub heading_probe_note: Handle<Image>,
    pub sun_mask_debug: Handle<Image>,
}

/// Everything the handles hold, tagged with the material generation they
/// were made for. Slots line up with the map's own tables; a slot is empty
/// when its image never loaded.
#[derive(Clone, Debug, Default)]
pub struct LiveImageHandlesData {
    pub revision_stamp: MaterialEpochId,
    pub shader_textures: Vec<Option<Handle<Image>>>,
    pub surface_titles: Vec<String>,
    pub probe_images: Vec<Option<Handle<Image>>>,
    pub lightmaps: Vec<Option<LiveLightbakeHandles>>,
    pub model_light: Option<Handle<Image>>,
}

/// The loaded map's image handles, shared by reference: cloning is cheap and
/// [`same_handles`](Self::same_handles) says whether two clones are the same set.
#[derive(Resource, Clone, Debug, Default)]
pub struct LiveImageHandles(Arc<LiveImageHandlesData>);

impl Deref for LiveImageHandles {
    type Target = LiveImageHandlesData;

    fn deref(&self) -> &LiveImageHandlesData {
        &self.0
    }
}

impl LiveImageHandles {
    pub fn build_from_stores(
        revision_stamp: MaterialEpochId,
        shader_textures: Vec<Option<Handle<Image>>>,
        surface_titles: Vec<String>,
        probe_imgs: Vec<Option<Handle<Image>>>,
        lightmaps: Vec<Option<LiveLightbakeHandles>>,
        model_light_tex: Option<Handle<Image>>,
    ) -> Self {
        Self(Arc::new(LiveImageHandlesData {
            revision_stamp,
            shader_textures,
            surface_titles,
            probe_images: probe_imgs,
            lightmaps,
            model_light: model_light_tex,
        }))
    }

    /// Whether `other` is this very set, not just an equal one.
    pub fn same_handles(&self, rhs: &Self) -> bool {
        Arc::ptr_eq(&self.0, &rhs.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_are_the_same_set_and_a_rebuild_is_not() {
        let handles = LiveImageHandles::default();
        assert!(handles.same_handles(&handles.clone()));
        assert!(!handles.same_handles(&LiveImageHandles::default()));
    }

    #[test]
    fn the_pools_come_back_through_the_deref() {
        let handles = LiveImageHandles::build_from_stores(
            MaterialEpochId::default(),
            vec![None, None],
            vec!["a".into(), "b".into()],
            Vec::new(),
            Vec::new(),
            None,
        );
        assert_eq!(handles.shader_textures.len(), 2);
        assert_eq!(handles.surface_titles[1], "b");
        assert!(handles.model_light.is_none());
    }
}
