//! Read access to code constants and textures, from one source or two layered.

mod store;

use std::sync::Arc;

pub use store::RuntimeCodeSources;

use crate::RuntimeImageId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeSourceError {
    TextureIndexOverflow,
}

pub trait CodeSourceLookup {
    fn constant_arc(&self, index: u16) -> Option<Arc<[[u32; 4]]>>;
    fn texture(&self, index: u32) -> Option<u8>;
    fn texture_image(&self, index: u32) -> Option<RuntimeImageId>;
}

impl CodeSourceLookup for RuntimeCodeSources {
    fn constant_arc(&self, index: u16) -> Option<Arc<[[u32; 4]]>> {
        RuntimeCodeSources::constant_arc(self, index)
    }

    fn texture(&self, index: u32) -> Option<u8> {
        RuntimeCodeSources::texture(self, index)
    }

    fn texture_image(&self, index: u32) -> Option<RuntimeImageId> {
        RuntimeCodeSources::texture_image(self, index)
    }
}

/// Two sources read as one; the overlay wins wherever it has a value.
pub struct LayeredCodeSources<'a> {
    pub base: &'a RuntimeCodeSources,
    pub overlay: &'a RuntimeCodeSources,
}

impl CodeSourceLookup for LayeredCodeSources<'_> {
    fn constant_arc(&self, index: u16) -> Option<Arc<[[u32; 4]]>> {
        let top = self.overlay.constant_arc(index);
        top.or_else(|| self.base.constant_arc(index))
    }

    fn texture(&self, index: u32) -> Option<u8> {
        let top = self.overlay.texture(index);
        top.or_else(|| self.base.texture(index))
    }

    fn texture_image(&self, index: u32) -> Option<RuntimeImageId> {
        let source = match self.overlay.texture(index) {
            Some(_) => self.overlay,
            None => self.base,
        };
        source.texture_image(index)
    }
}
