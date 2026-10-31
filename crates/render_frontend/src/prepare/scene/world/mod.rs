//! The loaded world as the frontend holds it: draw batches, visibility data,
//! static models, lights and the authored placements.

mod bmodel;
mod build;
mod dpvs;
mod model_set;
mod scene;
mod daylight;

pub use build::level_stage_from_draw;
pub use dpvs::{LevelHide, LevelSight, LevelDrawItem, LevelDrawItemKind, SelfContainedVisibilityPortal};
pub use model_set::*;
pub use render_frame::{LightFalloffBind, LegacyFalloffParams};
pub use render_scene::WorldEyePlacement;
pub use scene::{MapDirKeyLight, PROP_ILLUM_MAX_CLIENT_VIEWS, LevelStage};

pub(crate) use bmodel::{posed_solid_bounds, unit_rotation};
