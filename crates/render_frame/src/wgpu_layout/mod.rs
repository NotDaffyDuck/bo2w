//! Turning a pass's shader interface into the vertex-buffer and bind-group
//! layouts the GPU backend creates.

mod derive;
mod model;
mod table;

pub use self::derive::derive_wgpu_pass_layout;
pub use self::model::{
    TextureTableDimension, WgpuBindLayoutEntry, WgpuBindingKind, WgpuLayoutRefusal, WgpuPassLayout, WgpuShaderVisibility,
    WgpuVertexAttribute, WgpuVertexBufferLayout, WgpuVertexFormat,
};
pub use self::table::{
    TEXTURE_TABLE_2D_CAPACITY, TEXTURE_TABLE_3D_CAPACITY, TEXTURE_TABLE_CUBE_CAPACITY,
    TEXTURE_TABLE_SAMPLER_CAPACITY, texture_table_bind_entries,
};
