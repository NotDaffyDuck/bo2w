//! Draw surfaces. The per-frame lists of what gets drawn, together with the
//! state attached to each draw, from material setup to shadow casters.

// Materials and how a surface is lit.
pub mod lighting_draw_method;
pub mod material_run;
pub mod material_live;
pub mod pass_colour_space;
pub mod mesh_build;

// Frame-wide view state and post effects.
pub mod depth_span;
pub mod dof;
pub mod screen_grade_view;
pub mod fog;
pub mod frame_outputs;
pub mod gpu_handles;
pub mod material_setup;
pub mod sun_effects;

// Draw lists, the command context and the shadow casters.
pub mod cmd_context;
pub(crate) mod list;
pub mod spot_casters;
pub mod daylight_shadow;

mod lamp_receivers;
mod kept_lanes;
#[cfg(test)]
mod retained_golden_tests;
mod sun_shadow_clip;

pub use cmd_context::*;
pub use depth_span::*;
pub use self::screen_grade_view::*;
pub use frame_outputs::*;
pub use gpu_handles::*;
pub use list::*;
pub use lighting_draw_method::*;
pub use self::material_run::*;
pub use material_live::*;
pub use pass_colour_space::*;
pub use kept_lanes::*;
pub use material_setup::*;
pub use spot_casters::*;
pub use sun_effects::SunEffectsFrameInput;
pub(crate) use sun_effects::publish_sun_effects_frame;
pub use self::daylight_shadow::*;
pub use self::mesh_build::*;
