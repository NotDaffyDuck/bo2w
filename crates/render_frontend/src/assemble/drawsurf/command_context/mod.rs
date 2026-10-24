//! What the shaders read from the frame: the code-constant register numbers,
//! the per-map resources that feed them, and the systems that fill them in each
//! frame before anything is drawn.

mod produce;
mod registers;
mod resources;
mod system;

pub use produce::*;
pub use registers::*;
pub use resources::*;
pub(crate) use system::update_command_context_code_sources;

pub use super::fog::MapFrameFog;
pub use crate::prepare::scene::view_parms::{host_clip_from_view, pack_live_view_parms};
pub use crate::prepare::scene::world::{
    LightAttenuationBind, MapDirPrimaryLight, T5LightFalloffPack,
};
pub use render_frame::code_math::{
    camera_relative_view_projection, code_transpose_matrix_row4, code_transpose_matrix_rows,
    float4_bits,
};
