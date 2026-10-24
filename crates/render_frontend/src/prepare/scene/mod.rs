//! What the frontend prepares from the loaded world each frame: the camera,
//! visibility, model lighting, and the world's GPU data.

pub mod camera;
pub mod cell_frustum_cmds;
pub mod cull;
pub mod frustum;
pub mod gfx_scene;
pub mod model_lighting_atlas;
pub mod model_lighting_cache;
pub mod smodel_geom_cache;
pub mod smodel_lighting;
pub mod spawn;
pub mod view_parms;
pub mod world;
pub mod world_gpu;
pub mod world_images;
pub mod world_occupancy;
pub mod world_plan;
