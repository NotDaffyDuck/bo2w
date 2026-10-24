//! Vertex layouts shared with the GPU, and the per-surface sampler selectors
//! that ride along with a draw.

mod gpu_vertices;
mod surface_ids;

pub use self::gpu_vertices::{PackedVertexRefusal, SmodelVertex, WorldVertex, WorldVertexRefusal};
pub use self::surface_ids::{
    DYNAMIC_INDEX_BUFFER_CAPACITY, SurfaceLightmapId, SurfaceReflectionProbeId,
    SurfaceSamplerInputs, xmodel_tess_info_packed_arm, xmodel_tess_info_vert_decl_type,
};
