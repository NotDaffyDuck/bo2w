//! World-geometry records.

/// One world-geometry draw: a triangle run inside the world index buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GfxTrianglesListEntry {
    /// Sort key combined with the backend setup bits.
    pub sort_key: u32,
    /// Triangles to draw.
    pub tri_count: u16,
    /// Index of the run's first index in the world index buffer.
    pub base_index: u32,
    /// Vertex the indices are relative to.
    pub first_vertex: u32,
    /// How many vertices the run spans.
    pub vertex_count: u32,
}
