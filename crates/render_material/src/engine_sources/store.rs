//! Storage for the engine-supplied constants and textures a pass can read.

use std::sync::Arc;

use super::EngineSourceError;
use crate::LiveImageId;

/// Code constants by index, plus per-index sampler state and bound image for
/// code textures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveEngineSources {
    constants: Vec<Option<Arc<[[u32; 4]]>>>,
    sampler_modes: Vec<Option<u8>>,
    bound_texture_slots: Vec<Option<LiveImageId>>,
}

impl LiveEngineSources {
    pub fn store_shader_constant(&mut self, index: u16, values: Vec<[u32; 4]>) {
        self.store_constant_vectors(index, &values);
    }

    /// Stores the rows, reusing the old allocation when nobody else holds it.
    pub fn store_constant_vectors(&mut self, index: u16, values: &[[u32; 4]]) {
        let index = usize::from(index);
        if index >= self.constants.len() {
            self.constants.resize(1 + index, None);
        }
        let cell = &mut self.constants[index];
        if let Some(held) = cell.as_mut()
            && held.len() == values.len()
            && let Some(sole) = Arc::get_mut(held)
        {
            sole.copy_from_slice(values);
        } else {
            *cell = Some(Arc::from(values));
        }
    }

    pub fn put_sampler_mode(&mut self, index: u32, texture_sampling_mode: u8) -> Result<(), EngineSourceError> {
        let index = usize::try_from(index).map_err(|_| EngineSourceError::TextureIndexTooLarge)?;
        if index >= self.sampler_modes.len() {
            self.sampler_modes.resize(1 + index, None);
            self.bound_texture_slots.resize(index + 1, None);
        }
        self.sampler_modes[index] = Some(texture_sampling_mode);
        self.bound_texture_slots[index] = None;
        Ok(())
    }

    pub fn shared_constants_for(&self, table_no: u16) -> Option<Arc<[[u32; 4]]>> {
        self.constants.get(usize::from(table_no))?.clone()
    }

    pub fn sampler_mode_of(&self, tex_no: u32) -> Option<u8> {
        let tex_no = usize::try_from(tex_no).ok()?;
        self.sampler_modes.get(tex_no).copied().flatten()
    }

    pub fn image_for(&self, tex_no: u32) -> Option<LiveImageId> {
        let tex_no = usize::try_from(tex_no).ok()?;
        self.bound_texture_slots.get(tex_no).copied().flatten()
    }
}
