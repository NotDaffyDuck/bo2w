//! The fixed-size pool that code-built meshes allocate vertices, indices and
//! argument slots from.

/// A bump allocator over three parallel arrays. Nothing is freed individually;
/// the only way back is [`shrink_code_mesh_verts`], which returns vertices from
/// the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshPoolBlock {
    pub vertices_consumed: u32,
    pub index_fill: u32,
    pub argument_fill: u32,
    pub vertex_budget: u32,
    pub index_budget: u32,
    pub argument_budget: u32,
    pub vertex_alert_level: u32,
    pub index_alert_level: u32,
    pub argument_alert_level: u32,
}

impl MeshPoolBlock {
    /// An empty pool with the standard capacities.
    pub fn engine_generated_mesh() -> Self {
        Self {
            vertices_consumed: 0,
            index_fill: 0,
            argument_fill: 0,
            vertex_budget: 0x4000,
            index_budget: 0x6000,
            argument_budget: 0x100,
            vertex_alert_level: 0x24,
            index_alert_level: 0x23,
            argument_alert_level: 0x25,
        }
    }
}

impl Default for MeshPoolBlock {
    fn default() -> Self {
        Self::engine_generated_mesh()
    }
}

/// Takes `count` vertices; the result is the first one's slot.
pub fn reserve_code_mesh_verts(mesh: &mut MeshPoolBlock, count: u32) -> Option<u16> {
    let end = mesh.vertices_consumed.wrapping_add(count);
    (end <= mesh.vertex_budget).then(|| std::mem::replace(&mut mesh.vertices_consumed, end) as u16)
}

/// Takes `count` indices; the result is the first one's slot.
pub fn reserve_code_mesh_indices(mesh: &mut MeshPoolBlock, count: u32) -> Option<u32> {
    let end = mesh.index_fill.wrapping_add(count);
    (end <= mesh.index_budget).then(|| std::mem::replace(&mut mesh.index_fill, end))
}

/// Gives back `count` vertices from the end of the pool.
pub fn shrink_code_mesh_verts(mesh: &mut MeshPoolBlock, count: u32) {
    mesh.vertices_consumed = mesh.vertices_consumed.wrapping_sub(count);
}

/// Reserves a whole mesh at once: `(first vertex, first index, first argument
/// slot)`. The argument slot is `None` when `args` is zero. Either everything
/// fits and all three counters advance, or nothing changes.
pub fn reserve_code_mesh(
    mesh: &mut MeshPoolBlock,
    vertex_points: u32,
    index_run_values: u32,
    args: u32,
) -> Option<(u16, u32, Option<u32>)> {
    let vert_end = mesh.vertices_consumed.wrapping_add(vertex_points);
    let index_end = mesh.index_fill.wrapping_add(index_run_values);
    let args_end = mesh.argument_fill.wrapping_add(args);
    let fits = vert_end <= mesh.vertex_budget
        && index_end <= mesh.index_budget
        && (args == 0 || args_end <= mesh.argument_budget);
    if !fits {
        return None;
    }
    let first_vert = std::mem::replace(&mut mesh.vertices_consumed, vert_end) as u16;
    let idx_start = std::mem::replace(&mut mesh.index_fill, index_end);
    let first_arg = (args != 0).then(|| std::mem::replace(&mut mesh.argument_fill, args_end));
    Some((first_vert, idx_start, first_arg))
}
