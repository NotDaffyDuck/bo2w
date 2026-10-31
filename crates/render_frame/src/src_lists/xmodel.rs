//! Dynamic-model records.

/// One dynamic-model draw. The layout is fixed at sixteen bytes; the byte after
/// `tessellation_info_flags` is padding the backend never reads.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RigidModelBatchRecord {
    /// Sort key combined with the backend setup bits.
    pub combined_key_bits: u32,
    /// Where the triangles' indices start, in bytes.
    pub index_data_start_bytes: u32,
    /// Triangles to draw.
    pub tri_tally: u16,
    /// Tessellation info the vertex stage reads from byte 10 of the record.
    pub tessellation_info_flags: u8,
    _filler_byte_b: u8,
    /// Handle to the lighting data the object reads.
    pub illum_handle: u32,
}

impl RigidModelBatchRecord {
    /// Builds a record with its padding zeroed.
    pub const fn new(
        combined_key_bits: u32,
        index_data_start_bytes: u32,
        tri_tally: u16,
        tessellation_info_flags: u8,
        illum_handle: u32,
    ) -> Self {
        Self {
            combined_key_bits,
            index_data_start_bytes,
            tri_tally,
            tessellation_info_flags,
            illum_handle,
            _filler_byte_b: 0,
        }
    }
}
