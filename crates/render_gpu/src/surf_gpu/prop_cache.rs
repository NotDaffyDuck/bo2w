//! GPU copies of the static-model pipes: one vertex buffer and one index buffer that the
//! pipes patches in place, plus a per-frame index buffer for the models drawn this frame.

use bevy::prelude::*;
use bevy::render::render_resource::{Buffer, BufferDescriptor, BufferUsages};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use lighting_t6::{
    PROP_CACHE_BANK_VB_BYTES, PROP_CACHE_INDEX_U16_TALLY, SMC_VERTEX_BYTES, PropCachePatchLock, smc_stream_source_byte_offset,
};
use render_frame::LIVE_INDEX_BUFFER_CAP;

use crate::surf_gpu::gpu_bufs::{padded_len, write_padded};

/// Bytes in one `u16` index.
const INDEX_BYTES: usize = std::mem::size_of::<u16>();

/// Size in bytes of the pipes's whole index buffer.
const CACHE_INDEX_BYTES: usize = PROP_CACHE_INDEX_U16_TALLY * INDEX_BYTES;

/// Size in bytes of the per-frame index buffer.
const FRAME_INDEX_BYTES: usize = LIVE_INDEX_BUFFER_CAP as usize * INDEX_BYTES;

/// Why a patch was not written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropCachePatchWriteReject {
    NoBuffer,
    LockRefused,
}

/// The three buffers, created together by `ensure`.
#[derive(Resource, Default)]
pub struct PropCacheGpu {
    verts: Option<Buffer>,
    indices: Option<Buffer>,
    frame_indices: Option<Buffer>,
    /// Layout revision whose frame indices are already on the GPU.
    frame_revision: Option<u64>,
}

/// A buffer the gpu_queue can write into.
fn writable_buffer(render_device: &RenderDevice, buf_name: &'static str, byte_len: usize, kind: BufferUsages) -> Buffer {
    let descriptor = BufferDescriptor { label: Some(buf_name), size: byte_len as u64, usage: BufferUsages::COPY_DST | kind, mapped_at_creation: false };
    render_device.create_buffer(&descriptor)
}

impl PropCacheGpu {
    /// Creates whichever of the three buffers does not exist yet.
    pub fn ensure_ready(&mut self, render_device: &RenderDevice) {
        let make = |name, bytes, kind| writable_buffer(render_device, name, bytes, kind);
        let vertex_bytes = SMC_VERTEX_BYTES as usize;
        self.verts.get_or_insert_with(|| make("bo2_smodel_cache_vertices", vertex_bytes, BufferUsages::VERTEX));
        self.indices.get_or_insert_with(|| make("bo2_smodel_cache_indices", CACHE_INDEX_BYTES, BufferUsages::INDEX));
        self.frame_indices.get_or_insert_with(|| make("bo2_smodel_frame_indices", FRAME_INDEX_BYTES, BufferUsages::INDEX));
    }

    pub fn vertex_buffer(&self) -> Option<&Buffer> { self.verts.as_ref() }

    pub fn index_buffer(&self) -> Option<&Buffer> { self.indices.as_ref() }

    /// The per-frame index list `write_dynamic_indices` fills.
    pub fn dynamic_index_buffer(&self) -> Option<&Buffer> { self.frame_indices.as_ref() }

    /// Uploads this frame's index list unless that layout revision is already uploaded.
    /// Returns true only when the buffer already holds the revision (nothing was written).
    pub fn write_dynamic_indices(&mut self, render_queue: &RenderQueue, list: &[u16], layout_edition: u64) -> bool {
        // Revision 0 means "no revision": always upload.
        if layout_edition != 0 && self.frame_revision == Some(layout_edition) {
            return true;
        }
        let Some(target) = self.frame_indices.as_ref() else {
            return false;
        };
        let raw: &[u8] = bytemuck::cast_slice(list);
        if !raw.is_empty() {
            if padded_len(raw.len()) > FRAME_INDEX_BYTES {
                return false;
            }
            write_padded(render_queue, target, raw);
        }
        self.frame_revision = Some(layout_edition);
        false
    }

    /// Byte span of one bank inside the pipes's vertex buffer.
    pub fn bank_byte_range(cache_slot_no: u16) -> Option<(u64, u64)> {
        let first_ix = u64::from(smc_stream_source_byte_offset(cache_slot_no)?);
        Some((first_ix, first_ix + u64::from(PROP_CACHE_BANK_VB_BYTES)))
    }

    /// Writes a vertex patch the pipes has locked.
    pub fn patch(&self, render_queue: &RenderQueue, patch_lock: PropCachePatchLock, data: &[u8]) -> Result<(), PropCachePatchWriteReject> {
        let vtx_list = self.verts.as_ref().ok_or(PropCachePatchWriteReject::NoBuffer)?;
        if !patch_lock.fits_byte_length(data.len()) {
            return Err(PropCachePatchWriteReject::LockRefused);
        }
        render_queue.write_buffer(vtx_list, u64::from(patch_lock.start_byte), data);
        Ok(())
    }

    /// Writes an index patch at `byte_offset`, refusing anything past the end of the buffer.
    pub fn patch_indices(&self, render_queue: &RenderQueue, at: u32, data: &[u8]) -> Result<(), PropCachePatchWriteReject> {
        let indices = self.indices.as_ref().ok_or(PropCachePatchWriteReject::NoBuffer)?;
        let fits = (at as usize).checked_add(data.len()).is_some_and(|end| end <= CACHE_INDEX_BYTES);
        if !fits {
            return Err(PropCachePatchWriteReject::LockRefused);
        }
        render_queue.write_buffer(indices, u64::from(at), data);
        Ok(())
    }
}
