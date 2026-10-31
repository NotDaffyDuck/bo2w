//! What each frame is put together from: who stands in the scene, where the
//! eye is and what it sees, which models are still waiting for light.
//! Producers write these; the frontend and the GPU side read them.

mod camera;
mod dpvs_cells;
mod dyn_atpoint;
mod eye_parms;
mod frustum;
mod host;
mod illum_requests;
mod model_lighting_atlas;
mod plugin;
mod present;
mod smodel;
mod visual_stage;
mod world_instances;
mod xmodel_draw;

pub use camera::*;
pub use dpvs_cells::*;
pub use dyn_atpoint::*;
pub use eye_parms::*;
pub use frustum::*;
pub use host::*;
pub use illum_requests::*;
pub use model_lighting_atlas::*;
pub use plugin::*;
pub use present::*;
pub use smodel::*;
pub use visual_stage::*;
pub use world_instances::*;
pub use xmodel_draw::*;

