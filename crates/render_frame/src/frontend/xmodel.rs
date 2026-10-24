//! Dynamic-model records.

/// One dynamic-model draw. The layout is fixed at sixteen bytes; the byte after
/// `tess_info_byte_10` is padding the backend never reads.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GfxXModelRigidEntry {
    /// Sort key combined with the backend setup bits.
    pub packed_key: u32,
    /// Where the triangles' indices start, in bytes.
    pub index_byte_offset: u32,
    /// Triangles to draw.
    pub tri_count: u16,
    /// Tessellation info the vertex stage reads from byte 10 of the record.
    pub tess_info_byte_10: u8,
    _pad_0b: u8,
    /// Handle to the lighting data the object reads.
    pub lighting_handle: u32,
}

impl GfxXModelRigidEntry {
    /// Builds a record with its padding zeroed.
    pub const fn new(
        packed_key: u32,
        index_byte_offset: u32,
        tri_count: u16,
        tess_info_byte_10: u8,
        lighting_handle: u32,
    ) -> Self {
        Self {
            packed_key,
            index_byte_offset,
            tri_count,
            tess_info_byte_10,
            lighting_handle,
            _pad_0b: 0,
        }
    }
}
