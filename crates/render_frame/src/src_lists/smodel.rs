//! Static-model records.

/// Bytes between consecutive static-model entries in the backend's view.
pub const STATIC_FIXED_MESH_RECORD_BYTES: usize = 0x10;

/// A slice of the shared index pool, in indices, handed out to one static model
/// by the pretessellation stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropPrebuiltRange {
    /// Position of the first index in the pool.
    pub start: u32,
    /// How many indices the model owns.
    pub count: u32,
}

/// One static-model draw. `index_data_start_bytes` is a byte position in the 16-bit
/// index pool, not an element index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticModelRigidRecord {
    /// Sort key combined with the backend setup bits.
    pub combined_key_bits: u32,
    /// Triangles to draw.
    pub tri_tally: u16,
    /// Where the triangles' indices start, in bytes.
    pub index_data_start_bytes: u32,
    /// Handle to the lighting data the instance reads.
    pub illum_handle: u16,
}
