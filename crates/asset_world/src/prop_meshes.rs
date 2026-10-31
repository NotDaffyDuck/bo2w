//! A model as the renderer draws it: its surfaces by level of detail, and the
//! packed rows those surfaces were built from.

pub use asset_model::{MeshRole, mesh_role_for};
use bevy::prelude::Mesh;

/// One drawable surface of a model.
#[derive(Clone, Debug)]
pub struct MeshFaceDrawCall {
    pub face_mesh: Mesh,
    pub mat_slot: Option<usize>,
    /// The surface's rows exactly as the model file packs them.
    pub compact_vertices: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
    /// The model surface this came from, counted from 1 so that 0 can mean none.
    pub figure_part_plus_1: Option<u8>,
    /// Where the surface's triangles start in its LOD's index list.
    pub figure_part_base_index: u16,
    /// Where the surface's vertices start in its LOD's vertex list.
    pub figure_part_vertex_offset: u16,
    pub collision: XSurfaceCollisionPayload,
}

/// What a surface offers for hit tests. Nothing is read from T6 models yet.
#[derive(Clone, Debug)]
pub enum XSurfaceCollisionPayload {
    Unavailable { origin_layout: &'static str },
}

/// A model's packed vertex rows, when it has them.
#[derive(Clone, Debug)]
pub enum PackedVertexPayload {
    T6(Vec<[u8; asset_t6::T6_PACKED_VERTEX]>),
    Unavailable { origin_layout: &'static str },
}

const NOT_INSTALLED: &str = "packed vertex payload not installed";

impl Default for PackedVertexPayload {
    fn default() -> Self {
        Self::Unavailable { origin_layout: NOT_INSTALLED }
    }
}

/// All the surfaces of one model, grouped into its four levels of detail.
#[derive(Clone, Debug)]
pub struct StaticMeshData {
    pub name: String,
    pub detail_facets: [Vec<MeshFaceDrawCall>; 4],
    pub vertex_total: usize,
    pub triangle_total: usize,
    /// The surface-material-count byte of each LOD.
    pub detail_prop_cache: Option<[[u8; 4]; 4]>,
    pub lod: Option<asset_model::ModelDetailSelector>,
}

impl StaticMeshData {
    /// Surfaces over every LOD.
    pub fn facet_tally(&self) -> usize {
        self.detail_facets.iter().map(Vec::len).sum()
    }

    /// The surfaces of the finest LOD.
    pub fn surfaces(&self) -> &[MeshFaceDrawCall] {
        self.detail_facets[0].as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;

    fn surface() -> MeshFaceDrawCall {
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        MeshFaceDrawCall {
            face_mesh: mesh,
            mat_slot: Default::default(),
            compact_vertices: vec![],
            figure_part_plus_1: Default::default(),
            figure_part_base_index: Default::default(),
            figure_part_vertex_offset: Default::default(),
            collision: XSurfaceCollisionPayload::Unavailable { origin_layout: "none" },
        }
    }

    #[test]
    fn surfaces_are_counted_over_every_lod_but_listed_for_the_first() {
        let model = StaticMeshData {
            name: "crate".into(),
            detail_facets: [2, 1, 0, 1].map(|n| (0..n).map(|_| surface()).collect()),
            vertex_total: Default::default(),
            triangle_total: Default::default(),
            detail_prop_cache: Default::default(),
            lod: Default::default(),
        };
        assert_eq!(model.facet_tally(), 4);
        assert_eq!(model.surfaces().len(), 2);
    }

    #[test]
    fn rows_that_were_never_installed_say_so() {
        match PackedVertexPayload::default() {
            PackedVertexPayload::Unavailable { origin_layout } => {
                assert_eq!(origin_layout, NOT_INSTALLED)
            }
            PackedVertexPayload::T6(held) => panic!("a default payload holds no rows, got {}", held.len()),
        }
    }
}
