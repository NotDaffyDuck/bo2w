//! Which retained draws each spot-shadow light renders, and the packed lists
//! the shadow pass reads them through.

mod fill;
mod lights;
mod lookup;
mod plan;

pub use fill::fill_spot_shadow_caster_plan;
pub use plan::SpotShadowCasterPlan;
pub use render_frame::SpotShadowFrameSlot;
