//! Pure material rules: pass setup, vertex routing, the runtime catalog,
//! prepared tables and the code-constant banks.

mod argument;
mod catalog;
pub mod engine_sources;
pub mod decl_table;
mod depth_state;
mod execute;
pub mod pass_abi;
mod pass_color_space;
mod polygon_offset;
mod prepared;
mod setup;
mod stage;
mod vertex_decl;
mod vertex_layout;

pub use argument::LiveArgumentBinding;
pub use catalog::*;
pub use engine_sources::*;
pub use d3d9_sm3::SUPPORTED_OP_FACET;
pub use d3d9_sm3::SamplerTexShape;
pub use depth_state::*;
pub use execute::*;
pub use pass_abi::ConstantOrigin;
pub use pass_abi::PassShaderContract;
pub use pass_abi::SamplerOrigin;
pub use pass_abi::PassVertexInput;
pub use pass_color_space::PassColorEncoding;
pub use pass_color_space::pass_color_space;
pub use polygon_offset::*;
pub use prepared::*;
pub use setup::SetupVariantBranch;
pub use setup::MethodType;
pub use stage::LiveShaderStage;
pub use vertex_decl::LiveVertexFormat;
pub use vertex_layout::LEGACY_WORLD_LAYER_VERTEX_BYTES;
pub use vertex_layout::VertexFormatLineage;
