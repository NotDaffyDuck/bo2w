//! Storage for the engine-supplied constants and textures a pass can read.

use std::sync::Arc;

use super::CodeSourceError;
use crate::RuntimeImageId;

type ConstantRows = Arc<[[u32; 4]]>;

/// Code constants by index, plus per-index sampler state and bound image for
/// code textures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeCodeSources {
    constants: Vec<Option<ConstantRows>>,
    textures: Vec<Option<u8>>,
    texture_images: Vec<Option<RuntimeImageId>>,
}

impl RuntimeCodeSources {
    pub fn set_constant(&mut self, index: u16, rows: Vec<[u32; 4]>) {
        self.set_constant_rows(index, &rows);
    }

    /// Stores the rows, reusing the old allocation when nobody else holds it.
    pub fn set_constant_rows(&mut self, index: u16, rows: &[[u32; 4]]) {
        let index = usize::from(index);
        if self.constants.len() <= index {
            self.constants.resize(index + 1, None);
        }
        let slot = &mut self.constants[index];
        if let Some(held) = slot.as_mut()
            && held.len() == rows.len()
            && let Some(unique) = Arc::get_mut(held)
        {
            unique.copy_from_slice(rows);
        } else {
            *slot = Some(Arc::from(rows));
        }
    }

    pub fn set_texture(&mut self, index: u32, sampler_state: u8) -> Result<(), CodeSourceError> {
        let index = usize::try_from(index).map_err(|_| CodeSourceError::TextureIndexOverflow)?;
        if self.textures.len() <= index {
            self.textures.resize(index + 1, None);
            self.texture_images.resize(index + 1, None);
        }
        self.textures[index] = Some(sampler_state);
        self.texture_images[index] = None;
        Ok(())
    }

    pub fn constant_arc(&self, index: u16) -> Option<ConstantRows> {
        self.constants.get(usize::from(index))?.clone()
    }

    pub fn texture(&self, index: u32) -> Option<u8> {
        let index = usize::try_from(index).ok()?;
        self.textures.get(index).copied().flatten()
    }

    pub fn texture_image(&self, index: u32) -> Option<RuntimeImageId> {
        let index = usize::try_from(index).ok()?;
        self.texture_images.get(index).copied().flatten()
    }
}
