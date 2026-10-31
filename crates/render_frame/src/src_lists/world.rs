//! World-geometry records.

/// One world-geometry draw: a triangle run inside the world index buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriangleRunEntry {
    /// Sort key combined with the backend setup bits.
    pub setup_sort: u32,
    /// Triangles to draw.
    pub tri_tally: u16,
    /// Index of the run's first index in the world index buffer.
    pub index_start: u32,
    /// Vertex the indices are relative to.
    pub vertex_base: u32,
    /// How many vertices the run spans.
    pub vertex_count: u32,
}
