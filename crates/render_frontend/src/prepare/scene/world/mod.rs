//! The loaded world as the frontend holds it: draw batches, visibility data,
//! static models, lights and the authored placements.

mod bmodel;
mod build;
mod dpvs;
mod models;
mod scene;
mod sun;

pub use build::world_scene_from_draw;
pub use dpvs::{WorldCull, WorldDpvs, WorldDrawItem, WorldDrawItemKind, WorldPortal};
pub use models::*;
pub use render_frame::{LightAttenuationBind, T5LightFalloffPack};
pub use render_scene::WorldCameraPose;
pub use scene::{MapDirPrimaryLight, SMODEL_LIGHTING_MAX_CLIENT_VIEWS, WorldScene};

pub(crate) use bmodel::{posed_brush_bounds, transform_from_gfx_placement};
