//! Pure material rules: pass setup, vertex routing, the runtime catalog,
//! prepared tables and the code-constant banks.

pub mod argument;
pub mod catalog;
pub mod code_sources;
pub mod decl_table;
mod depth_state;
pub mod execute;
pub mod pass_abi;
pub mod pass_color_space;
mod polygon_offset;
pub mod prepared;
pub mod setup;
pub mod stage;
pub mod vertex_decl;
pub mod vertex_layout;

pub use argument::RuntimeArgumentBinding;
pub use catalog::*;
pub use code_sources::*;
pub use d3d9_sm3::{SUPPORTED_OPCODE_SURFACE, SamplerTextureDimension};
pub use depth_state::*;
pub use execute::*;
pub use pass_abi::{ConstantSource, PassProgramAbi, SamplerSource, VertexAttribute};
pub use pass_color_space::{PassColorSpace, pass_color_space};
pub use polygon_offset::*;
pub use prepared::*;
pub use setup::{SetupArm, TechType};
pub use stage::RuntimeShaderStage;
pub use vertex_decl::RuntimeVertexDecl;
pub use vertex_layout::{T5_WORLD_LAYER_HOST_STRIDE, VertexLayoutFamily};
