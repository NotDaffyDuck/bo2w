//! A model as the renderer draws it: its surfaces by level of detail, and the
//! packed rows those surfaces were built from.

pub use asset_model::{ModelKind, model_kind};
use bevy::prelude::Mesh;

/// One drawable surface of a model.
#[derive(Clone, Debug)]
pub struct ModelSurfaceDraw {
    pub mesh: Mesh,
    pub material: Option<usize>,
    /// The surface's rows exactly as the model file packs them.
    pub packed_vertices: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
    /// The model surface this came from, counted from 1 so that 0 can mean none.
    pub xsurface_plus_1: Option<u8>,
    /// Where the surface's triangles start in its LOD's index list.
    pub xsurface_base_index: u16,
    /// Where the surface's vertices start in its LOD's vertex list.
    pub xsurface_vert_offset: u16,
    pub collision: XSurfaceCollisionPayload,
}

/// What a surface offers for hit tests. Nothing is read from T6 models yet.
#[derive(Clone, Debug)]
pub enum XSurfaceCollisionPayload {
    Unavailable { source_layout: &'static str },
}

/// A model's packed vertex rows, when it has them.
#[derive(Clone, Debug)]
pub enum PackedVertexPayload {
    T6(Vec<[u8; asset_t6::T6_PACKED_VERTEX]>),
    Unavailable { source_layout: &'static str },
}

impl Default for PackedVertexPayload {
    fn default() -> Self {
        Self::Unavailable {
            source_layout: "packed vertex payload not installed",
        }
    }
}

/// All the surfaces of one model, grouped into its four levels of detail.
#[derive(Clone, Debug)]
pub struct ModelMesh {
    pub name: String,
    pub lod_surfaces: [Vec<ModelSurfaceDraw>; 4],
    pub vertices: usize,
    pub triangles: usize,
    /// The surface-material-count byte of each LOD.
    pub lod_smc: Option<[[u8; 4]; 4]>,
    pub lod: Option<asset_model::ModelLodSelector>,
}

impl ModelMesh {
    /// Surfaces over every LOD.
    pub fn surface_count(&self) -> usize {
        self.lod_surfaces.iter().map(Vec::len).sum()
    }

    /// The surfaces of the finest LOD.
    pub fn surfaces(&self) -> &[ModelSurfaceDraw] {
        self.lod_surfaces[0].as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;

    fn surface() -> ModelSurfaceDraw {
        ModelSurfaceDraw {
            mesh: Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default()),
            material: None,
            packed_vertices: Vec::new(),
            xsurface_plus_1: None,
            xsurface_base_index: 0,
            xsurface_vert_offset: 0,
            collision: XSurfaceCollisionPayload::Unavailable { source_layout: "none" },
        }
    }

    #[test]
    fn surfaces_are_counted_over_every_lod_but_listed_for_the_first() {
        let model = ModelMesh {
            name: "crate".into(),
            lod_surfaces: [
                vec![surface(), surface()],
                vec![surface()],
                Vec::new(),
                vec![surface()],
            ],
            vertices: 0,
            triangles: 0,
            lod_smc: None,
            lod: None,
        };
        assert_eq!(model.surface_count(), 4);
        assert_eq!(model.surfaces().len(), 2);
    }

    #[test]
    fn rows_that_were_never_installed_say_so() {
        let PackedVertexPayload::Unavailable { source_layout } = PackedVertexPayload::default()
        else {
            panic!("a default payload holds no rows");
        };
        assert_eq!(source_layout, "packed vertex payload not installed");
    }
}
