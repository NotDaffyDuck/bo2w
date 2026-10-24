//! The fixed-size pool that code-built meshes allocate vertices, indices and
//! argument slots from.

/// A bump allocator over three parallel arrays. Nothing is freed individually;
/// the only way back is [`shrink_code_mesh_verts`], which returns vertices from
/// the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GfxMeshData {
    pub vert_used: u32,
    pub index_used: u32,
    pub args_used: u32,
    pub vert_cap: u32,
    pub index_cap: u32,
    pub args_cap: u32,
    pub warn_verts: u32,
    pub warn_inds: u32,
    pub warn_args: u32,
}

impl GfxMeshData {
    /// An empty pool with the standard capacities.
    pub fn code_mesh() -> Self {
        Self {
            vert_used: 0,
            index_used: 0,
            args_used: 0,
            vert_cap: 0x4000,
            index_cap: 0x6000,
            args_cap: 0x100,
            warn_verts: 0x24,
            warn_inds: 0x23,
            warn_args: 0x25,
        }
    }
}

impl Default for GfxMeshData {
    fn default() -> Self {
        Self::code_mesh()
    }
}

/// Takes `count` vertices; the result is the first one's slot.
pub fn reserve_code_mesh_verts(mesh: &mut GfxMeshData, count: u32) -> Option<u16> {
    let end = mesh.vert_used.wrapping_add(count);
    (end <= mesh.vert_cap).then(|| std::mem::replace(&mut mesh.vert_used, end) as u16)
}

/// Takes `count` indices; the result is the first one's slot.
pub fn reserve_code_mesh_indices(mesh: &mut GfxMeshData, count: u32) -> Option<u32> {
    let end = mesh.index_used.wrapping_add(count);
    (end <= mesh.index_cap).then(|| std::mem::replace(&mut mesh.index_used, end))
}

/// Gives back `count` vertices from the end of the pool.
pub fn shrink_code_mesh_verts(mesh: &mut GfxMeshData, count: u32) {
    mesh.vert_used = mesh.vert_used.wrapping_sub(count);
}

/// Reserves a whole mesh at once: `(first vertex, first index, first argument
/// slot)`. The argument slot is `None` when `args` is zero. Either everything
/// fits and all three counters advance, or nothing changes.
pub fn reserve_code_mesh(
    mesh: &mut GfxMeshData,
    verts: u32,
    inds: u32,
    args: u32,
) -> Option<(u16, u32, Option<u32>)> {
    let vert_end = mesh.vert_used.wrapping_add(verts);
    let index_end = mesh.index_used.wrapping_add(inds);
    let args_end = mesh.args_used.wrapping_add(args);
    let fits = vert_end <= mesh.vert_cap
        && index_end <= mesh.index_cap
        && (args == 0 || args_end <= mesh.args_cap);
    if !fits {
        return None;
    }
    let first_vert = std::mem::replace(&mut mesh.vert_used, vert_end) as u16;
    let first_index = std::mem::replace(&mut mesh.index_used, index_end);
    let first_arg = (args != 0).then(|| std::mem::replace(&mut mesh.args_used, args_end));
    Some((first_vert, first_index, first_arg))
}
