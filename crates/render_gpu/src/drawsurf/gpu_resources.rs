use bevy::prelude::*;
use bevy::render::render_resource::Buffer;
use bevy::render::renderer::RenderQueue;

pub use assets::image_handles::{
    RuntimeImageHandles, RuntimeImageHandlesData, RuntimeLightmapHandles,
};

pub const LENS_VIEW_SIGNATURE: (bevy::render::render_resource::TextureFormat, u32) =
    (bevy::render::render_resource::TextureFormat::Rgba8Unorm, 1);

#[derive(Resource, Clone, Debug, Default)]
pub struct GpuSubmitReady {
    pub world_generation: frame::WorldGeneration,

    pub warm_pipelines: bool,

    pub overlay_gpu_wait: bool,
    pub pipeline_world_materials: std::sync::Arc<std::collections::HashSet<u16>>,
    pub pipeline_smodel_materials: std::sync::Arc<std::collections::HashSet<u16>>,
    pub pipeline_demand_revision: u64,
}

pub const UPLOAD_GRANULE: usize = 64 * 1024;

pub fn padded_upload_len(len: usize) -> usize {
    len.next_multiple_of(UPLOAD_GRANULE)
}

pub fn write_buffer_padded(queue: &RenderQueue, buffer: &Buffer, bytes: &[u8]) {
    let Some(size) = std::num::NonZeroU64::new(padded_upload_len(bytes.len()) as u64) else {
        return;
    };
    let mut view = queue
        .write_buffer_with(buffer, 0, size)
        .expect("padded upload fits its buffer");
    view.slice(..bytes.len()).copy_from_slice(bytes);
    view.slice(bytes.len()..).fill(0);
}

pub fn write_buffer_range(queue: &RenderQueue, buffer: &Buffer, offset: u64, bytes: &[u8]) {
    let Some(size) = std::num::NonZeroU64::new(bytes.len() as u64) else {
        return;
    };
    debug_assert_eq!(
        offset % 4,
        0,
        "constant arena range offset is 4-byte aligned"
    );
    debug_assert_eq!(
        bytes.len() % 4,
        0,
        "constant arena range length is 4-byte aligned"
    );
    let mut view = queue
        .write_buffer_with(buffer, offset, size)
        .expect("range upload fits its buffer");
    view.copy_from_slice(bytes);
}

pub use super::image_registry::{
    ExtractedRuntimeImageHandles, RuntimeUploadedImageRegistry, RuntimeUploadedLightmapViews,
    UploadedTextureView, UploadedViewRefusal,
};
pub(crate) use super::image_registry::prepare_uploaded_image_registry;
