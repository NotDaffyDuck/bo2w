//! Animation clips and sets as the engine plays them.

mod anim_set;
mod anim_t6_import;
mod clip_mixer;

pub use anim_set::*;
pub use asset_core::*;
pub use clip_mixer::*;
pub use xmodel_runtime::*;

/// The clip types, under the name the animation and weapon code reaches them by.
pub mod motion_clip {
    pub use xmodel_runtime::*;
}
