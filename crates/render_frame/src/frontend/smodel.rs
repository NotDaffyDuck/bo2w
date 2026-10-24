//! Static-model records.

/// Bytes between consecutive static-model entries in the backend's view.
pub const SMODEL_RIGID_ENTRY_STRIDE: usize = 0x10;

/// A slice of the shared index pool, in indices, handed out to one static model
/// by the pretessellation stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmodelPretessRange {
    /// Position of the first index in the pool.
    pub start: u32,
    /// How many indices the model owns.
    pub count: u32,
}

/// One static-model draw. `index_byte_offset` is a byte position in the 16-bit
/// index pool, not an element index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GfxSmodelRigidEntry {
    /// Sort key combined with the backend setup bits.
    pub packed_key: u32,
    /// Triangles to draw.
    pub tri_count: u16,
    /// Where the triangles' indices start, in bytes.
    pub index_byte_offset: u32,
    /// Handle to the lighting data the instance reads.
    pub lighting_handle: u16,
}
