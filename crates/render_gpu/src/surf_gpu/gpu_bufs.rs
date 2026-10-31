//! What the GPU side shares with the rest of the renderer: the image handles it uploads
//! from, the readiness flags the frame waits on, and the two buffer-write helpers.

use std::collections::HashSet;
use std::num::NonZeroU64;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::render::render_resource::{Buffer, TextureFormat};
use bevy::render::renderer::RenderQueue;

pub use assets::surf_images::{
    LiveImageHandles, LiveImageHandlesData, LiveLightbakeHandles,
};
pub(crate) use super::image_registry::refresh_uploaded_images;
pub use super::image_registry::{
    ExtractedLiveImageHandles, LightmapPlane, LiveUploadedImageRegistry, UploadedTextureEye,
    UploadedEyeRejection,
};

/// Format and sample count a lens-view target is created with.
pub const LENS_EYE_FORMAT: (TextureFormat, u32) = (TextureFormat::Rgba8Unorm, 1);

/// What the submit step is waiting on, and which materials the pipelines are warmed for.
#[derive(Resource, Clone, Debug, Default)]
pub struct SubmitReadyGpu {
    pub level_generation: frame::LevelGeneration,
    pub prewarmed_pipelines: bool,
    pub topmost_gpu_wait: bool,
    pub pipeline_level_materials: Arc<HashSet<u16>>,
    pub pipeline_prop_materials: Arc<HashSet<u16>>,
    pub pipeline_demand_revision: u64,
}

/// Uploads that go through `write_buffer_padded` are rounded up to this many bytes.
pub const SEND_GRANULE: usize = 64 * 1024;

pub fn padded_len(len: usize) -> usize {
    len.next_multiple_of(SEND_GRANULE)
}

/// The size of a write of `len` bytes, or `None` when there is nothing to write.
fn write_size(len: usize) -> Option<NonZeroU64> {
    NonZeroU64::new(len as u64)
}

/// Writes `bytes` at the start of `buffer` and zero-fills the rest of the padded length, so
/// the tail never holds stale data from an earlier, longer upload.
pub fn write_padded(gpu_queue: &RenderQueue, dst: &Buffer, data: &[u8]) {
    let Some(size) = write_size(padded_len(data.len())) else {
        return;
    };
    let mut staging = gpu_queue
        .write_buffer_with(dst, 0, size)
        .expect("padded write stays inside the buffer");
    let used = data.len();
    staging.slice(..used).copy_from_slice(data);
    staging.slice(used..).fill(0);
}

/// Writes `bytes` at `offset`. Both must be multiples of four, as the gpu_queue requires.
pub fn write_range_into(gpu_queue: &RenderQueue, dst: &Buffer, at: u64, data: &[u8]) {
    let Some(size) = write_size(data.len()) else {
        return;
    };
    debug_assert!(at % 4 == 0, "range offset must be 4-byte aligned");
    debug_assert!(data.len() % 4 == 0, "range length must be 4-byte aligned");
    gpu_queue
        .write_buffer_with(dst, at, size)
        .expect("range write stays inside the buffer")
        .copy_from_slice(data);
}
