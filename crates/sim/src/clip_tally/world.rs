/// How much clip data the loaded map carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldClipCensus {
    pub brushes: u32,
    pub bsp_nodes: u32,
    pub bsp_leaves: u32,
    pub brush_leaf_indices: u32,
    pub mesh_tris: u32,
    pub cmodels: u32,
    pub prop_set: u32,
    /// Of the static models, those that bring collision triangles of their own.
    pub static_models_with_tris: u32,
    /// Whether the surface penetration table came with the map.
    pub wall_pierce_table_ready: bool,
}
