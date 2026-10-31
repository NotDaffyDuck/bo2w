//! Read access to code constants and textures, from one source or two layered.

mod store;

use std::sync::Arc;

pub use store::LiveEngineSources;

use crate::LiveImageId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineSourceError {
    TextureIndexTooLarge,
}

pub trait EngineValueSource {
    fn shared_constants_for(&self, table_no: u16) -> Option<Arc<[[u32; 4]]>>;
    fn sampler_mode_of(&self, tex_no: u32) -> Option<u8>;
    fn image_for(&self, tex_no: u32) -> Option<LiveImageId>;
}

impl EngineValueSource for LiveEngineSources {
    fn shared_constants_for(&self, table_no: u16) -> Option<Arc<[[u32; 4]]>> {
        LiveEngineSources::shared_constants_for(self, table_no)
    }

    fn sampler_mode_of(&self, tex_no: u32) -> Option<u8> {
        LiveEngineSources::sampler_mode_of(self, tex_no)
    }

    fn image_for(&self, tex_no: u32) -> Option<LiveImageId> {
        LiveEngineSources::image_for(self, tex_no)
    }
}

/// Two sources read as one; the overlay wins wherever it has a value.
pub struct StackedEngineInputs<'a> {
    pub base: &'a LiveEngineSources,
    pub overlay: &'a LiveEngineSources,
}

impl EngineValueSource for StackedEngineInputs<'_> {
    fn shared_constants_for(&self, table_no: u16) -> Option<Arc<[[u32; 4]]>> {
        let top = self.overlay.shared_constants_for(table_no);
        top.or_else(|| self.base.shared_constants_for(table_no))
    }

    fn sampler_mode_of(&self, tex_no: u32) -> Option<u8> {
        let top = self.overlay.sampler_mode_of(tex_no);
        top.or_else(|| self.base.sampler_mode_of(tex_no))
    }

    fn image_for(&self, tex_no: u32) -> Option<LiveImageId> {
        let source = match self.overlay.sampler_mode_of(tex_no) {
            Some(_) => self.overlay,
            None => self.base,
        };
        source.image_for(tex_no)
    }
}
