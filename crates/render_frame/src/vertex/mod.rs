//! Vertex layouts shared with the GPU, and the per-surface sampler selectors
//! that ride along with a draw.

mod gpu_vertices;
mod surface_ids;

pub use self::gpu_vertices::{PackedVertexRefusal, PropVertex, LevelVertex, WorldVertexRefusal};
pub use self::surface_ids::{
    LIVE_INDEX_BUFFER_CAP, SurfaceBakedLightId, SurfaceCubeProbeId,
    FacetSamplerInputs, model_tess_packed_arm, model_tess_vert_decl,
};
