//! Everything shaders pull from the frame as code sources: register numbers,
//! the per-map resources the values come from, and the writers and system
//! that refresh them before each frame is drawn.

mod produce;
mod registers;
mod resources;
mod system;

pub use super::fog::MapFrameHaze;
pub use crate::prepare::scene::eye_parms::{perspective_clip_matrix, encode_active_view_params};
pub use crate::{
    LightFalloffBind, MapDirKeyLight, LegacyFalloffParams,
};
pub use produce::*;
pub use registers::*;
pub use render_frame::view_math::eye_relative_clip;
pub use render_frame::view_math::flip_matrix_rows_to_words;
pub use render_frame::view_math::engine_transpose_matrix_rows;
pub use render_frame::view_math::vec4_bits;
pub use resources::*;
pub(crate) use system::update_command_context_engine_sources;
