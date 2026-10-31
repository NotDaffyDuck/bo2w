//! Which retained draws each spot-shadow light renders, and the packed lists
//! the shadow pass reads them through.

mod fill;
mod lamps;
mod find_in;
mod draw_layout;

pub use fill::fill_cone_shadow_caster_plan;
pub use draw_layout::ConeShadowCasterPlan;
pub use render_frame::ConeShadowFrameSlot;
