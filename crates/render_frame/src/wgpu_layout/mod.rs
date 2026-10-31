//! Turning a pass's shader interface into the vertex-buffer and bind-group
//! layouts the GPU backend creates.

mod derive;
mod model;
mod table;

pub use self::derive::plan_gpu_pass_bindings;
pub use self::model::{
    TextureTableDimension, GpuBindSlotDescriptor, GpuBindResourceKind, WgpuLayoutRejection, GpuPassBindingPlan, GpuStageVisibility,
    GpuVertexField, GpuVertexStreamLayout, GpuVertexElementKind,
};
pub use self::table::{
    FLAT_TEXTURE_TABLE_SLOTS, VOLUME_TEXTURE_TABLE_SIZE, CUBEMAP_TEXTURE_TABLE_SIZE,
    SAMPLER_TABLE_SIZE, table_bind_entries,
};
